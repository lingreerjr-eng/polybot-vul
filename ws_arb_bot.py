import asyncio
import json
import logging
import os
import re
import time
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

import requests
import websockets
from dotenv import load_dotenv

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import OrderArgs, OrderType, PostOrdersArgs
from py_clob_client.order_builder.constants import BUY

load_dotenv()

logger = logging.getLogger("ws_arb_bot")
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s | %(levelname)s | %(message)s",
)

# ---- Config ----
CLOB_HOST = os.getenv("CLOB_HOST", "https://clob.polymarket.com")

# Accept either:
# - WSS_URL="wss://ws-subscriptions-clob.polymarket.com"
# - WSS_URL="wss://ws-subscriptions-clob.polymarket.com/ws"
# - WSS_URL="wss://ws-subscriptions-clob.polymarket.com/ws/"
WSS_URL_RAW = os.getenv("WSS_URL", "wss://ws-subscriptions-clob.polymarket.com").strip()
WSS_BASE = re.sub(r"/ws/?$", "", WSS_URL_RAW.rstrip("/"))

MARKET_WS_URL = f"{WSS_BASE}/ws/market"
USER_WS_URL = f"{WSS_BASE}/ws/user"  # optional, not used unless you add user feed later

GAMMA_HOST = os.getenv("GAMMA_HOST", "https://gamma-api.polymarket.com")

PRIVATE_KEY = os.getenv("PRIVATE_KEY", "")
FUNDER_ADDRESS = os.getenv("FUNDER_ADDRESS", "")
SIGNATURE_TYPE = int(os.getenv("SIGNATURE_TYPE", "1"))

DRY_RUN = os.getenv("DRY_RUN", "true").lower() == "true"
SIZE = float(os.getenv("SIZE", "5"))
SUM_MAX = float(os.getenv("SUM_MAX", "0.97"))
STALE_SECONDS = float(os.getenv("STALE_SECONDS", "2.0"))
COOLDOWN_SECONDS = float(os.getenv("COOLDOWN_SECONDS", "1.0"))

# These are Gamma event slug prefixes, bot resolves the current window automatically
PREFIXES = {
    "BTC": "btc-updown-15m-",
    "ETH": "eth-updown-15m-",
    "SOL": "sol-updown-15m-",
    "XRP": "xrp-updown-15m-",
}


# ---- Data structures ----
@dataclass
class MarketPair:
    symbol: str
    slug: str
    condition_id: str
    token_a: str
    token_b: str


@dataclass
class TopOfBook:
    bid: Optional[Tuple[float, float]] = None  # (price, size)
    ask: Optional[Tuple[float, float]] = None  # (price, size)
    ts: float = 0.0


book: Dict[str, TopOfBook] = {}  # token_id -> TopOfBook
pair_by_symbol: Dict[str, MarketPair] = {}
symbol_by_token: Dict[str, str] = {}

_last_fire: Dict[str, float] = {}
_in_flight: Dict[str, bool] = {}


# ---- Helpers ----
def now_ts() -> float:
    return time.time()


def current_15m_start_epoch() -> int:
    t = int(time.time())
    return (t // 900) * 900


def _gamma_get_event_by_slug(slug: str) -> dict:
    url = f"{GAMMA_HOST}/events/slug/{slug}"
    r = requests.get(url, timeout=10)
    if r.status_code != 200:
        raise RuntimeError(f"Gamma {r.status_code} for slug={slug}: {r.text[:200]}")
    return r.json()


def _parse_clob_token_ids(raw_val) -> List[str]:
    # Gamma sometimes returns clobTokenIds as a JSON string, sometimes as a list.
    if raw_val is None:
        return []
    if isinstance(raw_val, list):
        return [str(x) for x in raw_val]
    raw = str(raw_val).strip()
    if raw.startswith("["):
        return [str(x) for x in json.loads(raw)]
    return [x.strip().strip('"') for x in raw.split(",") if x.strip()]


def resolve_current_pairs() -> Dict[str, MarketPair]:
    start = current_15m_start_epoch()
    resolved: Dict[str, MarketPair] = {}

    for sym, prefix in PREFIXES.items():
        candidates = [start, start + 900, start + 1800]
        last_err = None

        for ts0 in candidates:
            slug = f"{prefix}{ts0}"
            try:
                ev = _gamma_get_event_by_slug(slug)
                mkts = ev.get("markets") or []
                if not mkts:
                    raise RuntimeError("Gamma returned no markets")

                m0 = mkts[0]
                if not m0.get("acceptingOrders", True):
                    raise RuntimeError("market not acceptingOrders yet")

                condition_id = str(m0["conditionId"])
                token_ids = _parse_clob_token_ids(m0.get("clobTokenIds"))
                if len(token_ids) != 2:
                    raise RuntimeError(f"expected 2 token ids, got {len(token_ids)}")

                resolved[sym] = MarketPair(
                    symbol=sym,
                    slug=slug,
                    condition_id=condition_id,
                    token_a=str(token_ids[0]),
                    token_b=str(token_ids[1]),
                )
                break
            except Exception as e:
                last_err = e

        if sym not in resolved:
            logger.warning("resolve_failed | %s | %s", sym, last_err)

    return resolved


def init_clob_client() -> ClobClient:
    if not PRIVATE_KEY:
        raise RuntimeError("Missing PRIVATE_KEY in .env")

    if FUNDER_ADDRESS:
        client = ClobClient(
            host=CLOB_HOST,
            key=PRIVATE_KEY,
            chain_id=137,
            signature_type=SIGNATURE_TYPE,
            funder=FUNDER_ADDRESS,
        )
    else:
        client = ClobClient(host=CLOB_HOST, key=PRIVATE_KEY, chain_id=137)

    # This may try create then derive. Some accounts return 400 on create, then 200 on derive.
    client.set_api_creds(client.create_or_derive_api_creds())
    return client


def _best_bid_ask_from_levels(levels: List[dict], side: str) -> Optional[Tuple[float, float]]:
    # side="bid" => max price, side="ask" => min price
    if not levels:
        return None
    try:
        if side == "bid":
            best = max(levels, key=lambda r: float(r["price"]))
        else:
            best = min(levels, key=lambda r: float(r["price"]))
        return float(best["price"]), float(best["size"])
    except Exception:
        return None


def _set_tob_from_best_fields(tob: TopOfBook, msg: dict) -> bool:
    # Some messages include best_bid / best_ask fields directly
    bb = msg.get("best_bid") or msg.get("bestBid")
    ba = msg.get("best_ask") or msg.get("bestAsk")
    bbs = msg.get("best_bid_size") or msg.get("bestBidSize")
    bas = msg.get("best_ask_size") or msg.get("bestAskSize")

    changed = False
    if bb is not None and bbs is not None:
        tob.bid = (float(bb), float(bbs))
        changed = True
    if ba is not None and bas is not None:
        tob.ask = (float(ba), float(bas))
        changed = True
    return changed


def good_tob(t: Optional[TopOfBook]) -> bool:
    if not t or not t.ask:
        return False
    return (now_ts() - t.ts) <= STALE_SECONDS


async def post_batch_fok(
    client: ClobClient,
    sym: str,
    token_a: str,
    token_b: str,
    ask_a: float,
    ask_b: float,
) -> dict:
    if DRY_RUN:
        logger.warning(
            json.dumps(
                {
                    "event": "dry_run_skip",
                    "sym": sym,
                    "size": SIZE,
                    "a": {"token": token_a, "ask": ask_a},
                    "b": {"token": token_b, "ask": ask_b},
                    "ts": time.time(),
                }
            )
        )
        return {"dry_run": True}

    def _do_post() -> dict:
        logger.warning(json.dumps({"event": "order_submit", "sym": sym, "leg": "A", "token": token_a, "px": ask_a, "sz": SIZE, "ts": time.time()}))
        logger.warning(json.dumps({"event": "order_submit", "sym": sym, "leg": "B", "token": token_b, "px": ask_b, "sz": SIZE, "ts": time.time()}))

        orders = [
            PostOrdersArgs(
                order=client.create_order(
                    OrderArgs(
                        price=float(ask_a),
                        size=float(SIZE),
                        side=BUY,
                        token_id=str(token_a),
                    )
                ),
                orderType=OrderType.FOK,
            ),
            PostOrdersArgs(
                order=client.create_order(
                    OrderArgs(
                        price=float(ask_b),
                        size=float(SIZE),
                        side=BUY,
                        token_id=str(token_b),
                    )
                ),
                orderType=OrderType.FOK,
            ),
        ]

        resp = client.post_orders(orders)
        return resp

    resp = await asyncio.to_thread(_do_post)
    logger.warning(json.dumps({"event": "batch_result", "sym": sym, "resp": resp, "ts": time.time()})[:1200])
    return resp


async def maybe_fire(client: ClobClient, sym: str) -> None:
    pair = pair_by_symbol.get(sym)
    if not pair:
        return

    if _in_flight.get(sym, False):
        return

    last = _last_fire.get(sym, 0.0)
    if (now_ts() - last) < COOLDOWN_SECONDS:
        return

    ta, tb = pair.token_a, pair.token_b
    ba = book.get(ta)
    bb = book.get(tb)

    if not (good_tob(ba) and good_tob(bb)):
        return

    ask_a, size_a = ba.ask  # type: ignore
    ask_b, size_b = bb.ask  # type: ignore

    if size_a < SIZE or size_b < SIZE:
        return

    total = ask_a + ask_b
    if total > SUM_MAX:
        return

    _in_flight[sym] = True
    _last_fire[sym] = now_ts()

    logger.warning(
        json.dumps(
            {
                "event": "arb_trigger",
                "sym": sym,
                "slug": pair.slug,
                "sum": round(total, 6),
                "a": {"token": ta, "ask": ask_a, "sz": size_a},
                "b": {"token": tb, "ask": ask_b, "sz": size_b},
                "size": SIZE,
                "sum_max": SUM_MAX,
                "dry_run": DRY_RUN,
                "ts": time.time(),
            }
        )
    )

    try:
        resp = await post_batch_fok(client, sym, ta, tb, ask_a, ask_b)
        logger.info("exec_result | %s | %s", sym, json.dumps(resp)[:800])
    except Exception as e:
        logger.exception("exec_error | %s | %s", sym, e)
    finally:
        _in_flight[sym] = False


def update_book_from_msg(msg: dict) -> Optional[str]:
    # returns token_id if we updated it
    token_id = msg.get("asset_id") or msg.get("assetId") or msg.get("token_id") or msg.get("tokenId")
    if not token_id:
        return None
    token_id = str(token_id)

    tob = book.get(token_id) or TopOfBook()

    # Fast path: messages that already include best_bid/best_ask
    if _set_tob_from_best_fields(tob, msg):
        tob.ts = now_ts()
        book[token_id] = tob
        return token_id

    # Book snapshot detection (often includes bids/asks or buys/sells)
    is_book = (
        ("bids" in msg)
        or ("asks" in msg)
        or ("buys" in msg)
        or ("sells" in msg)
        or (msg.get("event_type") == "book")
        or (msg.get("type") == "book")
    )
    if is_book:
        bids = msg.get("bids") or msg.get("buys") or []
        asks = msg.get("asks") or msg.get("sells") or []

        best_bid = _best_bid_ask_from_levels(bids, "bid")
        best_ask = _best_bid_ask_from_levels(asks, "ask")

        if best_bid:
            tob.bid = best_bid
        if best_ask:
            tob.ask = best_ask

        tob.ts = now_ts()
        book[token_id] = tob
        return token_id

    # Some messages are bundles like {"market":..., "price_changes":[...]}.
    # We ignore these unless they include best_bid/best_ask fields (handled above).
    return None


async def ws_send_json(ws, obj: dict) -> None:
    await ws.send(json.dumps(obj))


async def ws_initial_subscribe(ws, token_ids: List[str]) -> None:
    # Official market subscription shape
    await ws_send_json(
        ws,
        {
            "assets_ids": token_ids,
            "type": "market",
            "custom_feature_enabled": True,
        },
    )


async def ws_update_subscription(ws, subscribe: List[str], unsubscribe: List[str]) -> None:
    # Official update ops
    if unsubscribe:
        await ws_send_json(
            ws,
            {
                "assets_ids": unsubscribe,
                "operation": "unsubscribe",
                "custom_feature_enabled": True,
            },
        )
    if subscribe:
        await ws_send_json(
            ws,
            {
                "assets_ids": subscribe,
                "operation": "subscribe",
                "custom_feature_enabled": True,
            },
        )


async def rollover_loop(ws) -> None:
    """Every 15 minutes, resolve the new 'current' markets and resubscribe tokens."""
    global pair_by_symbol, symbol_by_token

    last_slot = current_15m_start_epoch()
    while True:
        await asyncio.sleep(2)
        slot = current_15m_start_epoch()
        if slot == last_slot:
            continue
        last_slot = slot

        new_pairs = resolve_current_pairs()
        if not new_pairs:
            logger.warning("rollover_resolve_failed | no pairs")
            continue

        old_tokens = set(symbol_by_token.keys())

        pair_by_symbol = new_pairs
        symbol_by_token = {}
        for sym, p in pair_by_symbol.items():
            symbol_by_token[p.token_a] = sym
            symbol_by_token[p.token_b] = sym

        new_tokens = set(symbol_by_token.keys())

        to_unsub = sorted(list(old_tokens - new_tokens))
        to_sub = sorted(list(new_tokens - old_tokens))

        logger.info(
            "rollover | slot=%s | unsub=%d sub=%d",
            slot,
            len(to_unsub),
            len(to_sub),
        )
        await ws_update_subscription(ws, subscribe=to_sub, unsubscribe=to_unsub)


async def run_ws_loop(client: ClobClient, token_ids: List[str]) -> None:
    token_ids = [str(t) for t in sorted(list(set(token_ids)))]

    while True:
        try:
            logger.info("ws_connecting | url=%s | tokens=%d", MARKET_WS_URL, len(token_ids))

            async with websockets.connect(
                MARKET_WS_URL,
                ping_interval=None,  # we do app-level keepalive
                open_timeout=10,
                close_timeout=5,
                max_size=4_000_000,
            ) as ws:
                await ws_initial_subscribe(ws, token_ids)
                logger.info("ws_connected | url=%s", MARKET_WS_URL)

                # keepalive
                async def app_ping():
                    while True:
                        try:
                            await ws.send("PING")
                        except Exception:
                            return
                        await asyncio.sleep(10)

                asyncio.create_task(app_ping())
                asyncio.create_task(rollover_loop(ws))

                while True:
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=5)
                    except asyncio.TimeoutError:
                        logger.info("ws_idle | no messages in 5s, resending subscribe")
                        await ws_initial_subscribe(ws, token_ids)
                        continue

                    if raw == "PONG":
                        continue

                    # Too spammy at INFO. Flip to DEBUG if you want full firehose.
                    logger.debug("ws_msg | %s", str(raw)[:240])

                    try:
                        msg = json.loads(raw)
                    except Exception:
                        continue

                    items = msg if isinstance(msg, list) else [msg]
                    for item in items:
                        if not isinstance(item, dict):
                            continue

                        updated_token = update_book_from_msg(item)
                        if not updated_token:
                            continue

                        sym = symbol_by_token.get(updated_token)
                        if sym:
                            await maybe_fire(client, sym)

        except Exception as e:
            logger.exception("ws_loop_error | %s", e)
            await asyncio.sleep(2)


async def main() -> None:
    logger.info(
        "boot | dry_run=%s size=%s sum_max=%s stale=%s cooldown=%s",
        DRY_RUN,
        SIZE,
        SUM_MAX,
        STALE_SECONDS,
        COOLDOWN_SECONDS,
    )

    client = init_clob_client()

    pairs = resolve_current_pairs()
    if not pairs:
        raise RuntimeError("Could not resolve any current 15m markets via Gamma")

    global pair_by_symbol, symbol_by_token
    pair_by_symbol = pairs
    symbol_by_token = {}

    token_ids: List[str] = []
    for sym, p in pairs.items():
        token_ids.extend([p.token_a, p.token_b])
        symbol_by_token[p.token_a] = sym
        symbol_by_token[p.token_b] = sym
        logger.info("pair | %s | %s | %s", sym, p.slug, p.condition_id)

    await run_ws_loop(client, token_ids)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print()
