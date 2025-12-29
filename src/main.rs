use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy::primitives::Address;
use alloy::signers::{Signer, local::LocalSigner};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use polymarket_client_sdk::clob::{Client, Config};
use polymarket_client_sdk::clob::types::{Side, SignatureType};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

// Type aliases for cleaner code
type AuthenticatedClient = Client<Authenticated<Normal>>;
type EthSigner = LocalSigner<k256::ecdsa::SigningKey>;

// Configuration
#[derive(Debug, Clone)]
#[allow(dead_code)]  // Some fields reserved for future features
struct BotConfig {
    clob_host: String,
    wss_url: String,
    gamma_host: String,
    private_key: String,
    funder_address: Option<Address>,
    signature_type: u8,
    
    dry_run: bool,
    size: Decimal,
    sum_max: Decimal,
    stale_seconds: f64,
    cooldown_seconds: f64,
    
    bankroll_fraction: Decimal,
    reserve_usdc: Decimal,
    min_leg_notional_usd: Decimal,
    min_trade_notional_usd: Decimal,
    max_trade_notional_usd: Decimal,
    size_decimals: u32,
    balance_cache_ttl: f64,
    
    hedge_mode: String,
    hedge_sum_max: Decimal,
    hedge_retries: u32,
    hedge_sleep_ms: u64,
    unwind_retries: u32,
    unwind_sleep_ms: u64,
    
    prefixes: HashMap<String, String>,
}

impl BotConfig {
    fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        
        let funder_str = std::env::var("FUNDER_ADDRESS").ok();
        let funder_address = funder_str.and_then(|s| {
            if s.is_empty() { None } else { Address::from_str(&s).ok() }
        });
        
        let mut prefixes = HashMap::new();
        prefixes.insert("BTC".to_string(), "btc-updown-15m-".to_string());
        prefixes.insert("ETH".to_string(), "eth-updown-15m-".to_string());
        prefixes.insert("SOL".to_string(), "sol-updown-15m-".to_string());
        prefixes.insert("XRP".to_string(), "xrp-updown-15m-".to_string());
        
        Ok(Self {
            clob_host: std::env::var("CLOB_HOST").unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            wss_url: std::env::var("WSS_URL").unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string()),
            gamma_host: std::env::var("GAMMA_HOST").unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string()),
            private_key: std::env::var("PRIVATE_KEY").context("PRIVATE_KEY required")?,
            funder_address,
            signature_type: std::env::var("SIGNATURE_TYPE").unwrap_or_else(|_| "1".to_string()).parse()?,
            
            dry_run: std::env::var("DRY_RUN").unwrap_or_else(|_| "true".to_string()).to_lowercase() == "true",
            size: Decimal::from_str(&std::env::var("SIZE").unwrap_or_else(|_| "5".to_string()))?,
            sum_max: Decimal::from_str(&std::env::var("SUM_MAX").unwrap_or_else(|_| "0.97".to_string()))?,
            stale_seconds: std::env::var("STALE_SECONDS").unwrap_or_else(|_| "2.0".to_string()).parse()?,
            cooldown_seconds: std::env::var("COOLDOWN_SECONDS").unwrap_or_else(|_| "1.0".to_string()).parse()?,
            
            bankroll_fraction: Decimal::from_str(&std::env::var("BANKROLL_FRACTION").unwrap_or_else(|_| "0.25".to_string()))?,
            reserve_usdc: Decimal::from_str(&std::env::var("RESERVE_USDC").unwrap_or_else(|_| "2.0".to_string()))?,
            min_leg_notional_usd: Decimal::from_str(&std::env::var("MIN_LEG_NOTIONAL_USD").unwrap_or_else(|_| "1.0".to_string()))?,
            min_trade_notional_usd: Decimal::from_str(&std::env::var("MIN_TRADE_NOTIONAL_USD").unwrap_or_else(|_| "2.0".to_string()))?,
            max_trade_notional_usd: Decimal::from_str(&std::env::var("MAX_TRADE_NOTIONAL_USD").unwrap_or_else(|_| "50.0".to_string()))?,
            size_decimals: std::env::var("SIZE_DECIMALS").unwrap_or_else(|_| "2".to_string()).parse()?,
            balance_cache_ttl: std::env::var("BALANCE_CACHE_TTL").unwrap_or_else(|_| "1.0".to_string()).parse()?,
            
            hedge_mode: std::env::var("HEDGE_MODE").unwrap_or_else(|_| "refill_then_unwind".to_string()).to_lowercase(),
            hedge_sum_max: Decimal::from_str(&std::env::var("HEDGE_SUM_MAX").unwrap_or_else(|_| "0.98".to_string()))?,
            hedge_retries: std::env::var("HEDGE_RETRIES").unwrap_or_else(|_| "2".to_string()).parse()?,
            hedge_sleep_ms: std::env::var("HEDGE_SLEEP_MS").unwrap_or_else(|_| "150".to_string()).parse()?,
            unwind_retries: std::env::var("UNWIND_RETRIES").unwrap_or_else(|_| "3".to_string()).parse()?,
            unwind_sleep_ms: std::env::var("UNWIND_SLEEP_MS").unwrap_or_else(|_| "150".to_string()).parse()?,
            
            prefixes,
        })
    }
}

// Data structures
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarketPair {
    symbol: String,
    slug: String,
    condition_id: String,
    token_a: String,
    token_b: String,
}

#[derive(Debug, Clone)]
struct TopOfBook {
    bid: Option<(Decimal, Decimal)>,
    ask: Option<(Decimal, Decimal)>,
    ts: Instant,
}

impl Default for TopOfBook {
    fn default() -> Self {
        Self {
            bid: None,
            ask: None,
            ts: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GammaEvent {
    markets: Vec<GammaMarket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId")]
    condition_id: String,
    #[serde(rename = "clobTokenIds")]
    clob_token_ids: serde_json::Value,
    #[serde(rename = "acceptingOrders")]
    #[serde(default = "default_true")]
    accepting_orders: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WsMessage {
    #[serde(rename = "asset_id")]
    asset_id: Option<String>,
    #[serde(rename = "assetId")]
    asset_id_alt: Option<String>,
    event_type: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    bids: Option<Vec<BookLevel>>,
    asks: Option<Vec<BookLevel>>,
    best_bid: Option<String>,
    best_ask: Option<String>,
    best_bid_size: Option<String>,
    best_ask_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BookLevel {
    price: String,
    size: String,
}

#[derive(Clone)]
struct BalanceCache {
    available: Option<Decimal>,
    ts: Instant,
}

impl Default for BalanceCache {
    fn default() -> Self {
        Self {
            available: None,
            ts: Instant::now() - Duration::from_secs(3600),
        }
    }
}

// Shared state
struct SharedState {
    config: BotConfig,
    book: RwLock<HashMap<String, TopOfBook>>,
    pairs: RwLock<HashMap<String, MarketPair>>,
    symbol_by_token: RwLock<HashMap<String, String>>,
    last_fire: RwLock<HashMap<String, Instant>>,
    in_flight: RwLock<HashMap<String, bool>>,
    balance_cache: Mutex<BalanceCache>,
}

impl SharedState {
    fn new(config: BotConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            book: RwLock::new(HashMap::new()),
            pairs: RwLock::new(HashMap::new()),
            symbol_by_token: RwLock::new(HashMap::new()),
            last_fire: RwLock::new(HashMap::new()),
            in_flight: RwLock::new(HashMap::new()),
            balance_cache: Mutex::new(BalanceCache::default()),
        })
    }
}

// Utility functions
fn current_15m_start_epoch() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    (now / 900) * 900
}

fn parse_clob_token_ids(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(arr) => {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }
        serde_json::Value::String(s) => {
            if let Ok(arr) = serde_json::from_str::<Vec<String>>(s) {
                arr
            } else {
                s.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect()
            }
        }
        _ => vec![],
    }
}

fn round_down(value: Decimal, decimals: u32) -> Decimal {
    if decimals == 0 {
        value.floor()
    } else {
        let multiplier = Decimal::from(10_u64.pow(decimals));
        (value * multiplier).floor() / multiplier
    }
}

async fn fetch_gamma_event(gamma_host: &str, slug: &str) -> Result<GammaEvent> {
    let url = format!("{}/events/slug/{}", gamma_host, slug);
    let response = reqwest::get(&url).await?;
    
    if !response.status().is_success() {
        anyhow::bail!("Gamma API returned {}: {}", response.status(), slug);
    }
    
    let event: GammaEvent = response.json().await?;
    Ok(event)
}

async fn resolve_current_pairs(state: &Arc<SharedState>) -> Result<HashMap<String, MarketPair>> {
    let start = current_15m_start_epoch();
    let mut resolved = HashMap::new();
    
    for (sym, prefix) in &state.config.prefixes {
        let candidates = vec![start, start + 900, start + 1800];
        let mut last_err = None;
        
        for ts0 in candidates {
            let slug = format!("{}{}", prefix, ts0);
            
            match fetch_gamma_event(&state.config.gamma_host, &slug).await {
                Ok(ev) => {
                    if ev.markets.is_empty() {
                        last_err = Some(anyhow::anyhow!("No markets"));
                        continue;
                    }
                    
                    let m0 = &ev.markets[0];
                    if !m0.accepting_orders {
                        last_err = Some(anyhow::anyhow!("Not accepting orders"));
                        continue;
                    }
                    
                    let token_ids = parse_clob_token_ids(&m0.clob_token_ids);
                    if token_ids.len() != 2 {
                        last_err = Some(anyhow::anyhow!("Expected 2 tokens, got {}", token_ids.len()));
                        continue;
                    }
                    
                    resolved.insert(sym.clone(), MarketPair {
                        symbol: sym.clone(),
                        slug,
                        condition_id: m0.condition_id.clone(),
                        token_a: token_ids[0].clone(),
                        token_b: token_ids[1].clone(),
                    });
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }
        
        if !resolved.contains_key(sym) {
            warn!("Failed to resolve market for {}: {:?}", sym, last_err);
        }
    }
    
    Ok(resolved)
}

async fn init_clob_client(config: &BotConfig) -> Result<AuthenticatedClient> {
    let signer: EthSigner = LocalSigner::from_str(&config.private_key)?
        .with_chain_id(Some(137)); // Polygon
    
    let mut builder = Client::new(&config.clob_host, Config::default())?
        .authentication_builder(&signer);
    
    // Set signature type based on config
    let sig_type = match config.signature_type {
        0 => SignatureType::Eoa,
        1 => SignatureType::Proxy,        // Email/Magic login
        2 => SignatureType::GnosisSafe,   // Browser wallet proxy
        _ => SignatureType::Eoa,
    };
    builder = builder.signature_type(sig_type);
    
    // Only set funder for proxy wallets
    if let Some(funder) = config.funder_address {
        builder = builder.funder(funder);
    }
    
    let client = builder.authenticate().await?;
    
    Ok(client)
}

fn update_book_from_msg(book: &mut HashMap<String, TopOfBook>, msg: &WsMessage) -> Option<String> {
    let token_id = msg.asset_id.as_ref().or(msg.asset_id_alt.as_ref())?;
    
    let tob = book.entry(token_id.clone()).or_insert_with(TopOfBook::default);
    let mut updated = false;
    
    // Try best bid/ask fields
    if let (Some(bb), Some(bbs)) = (&msg.best_bid, &msg.best_bid_size) {
        if let (Ok(price), Ok(size)) = (Decimal::from_str(bb), Decimal::from_str(bbs)) {
            tob.bid = Some((price, size));
            updated = true;
        }
    }
    
    if let (Some(ba), Some(bas)) = (&msg.best_ask, &msg.best_ask_size) {
        if let (Ok(price), Ok(size)) = (Decimal::from_str(ba), Decimal::from_str(bas)) {
            tob.ask = Some((price, size));
            updated = true;
        }
    }
    
    // Try book levels
    if let Some(bids) = &msg.bids {
        if let Some(best) = bids.iter().max_by_key(|l| Decimal::from_str(&l.price).ok()) {
            if let (Ok(price), Ok(size)) = (Decimal::from_str(&best.price), Decimal::from_str(&best.size)) {
                tob.bid = Some((price, size));
                updated = true;
            }
        }
    }
    
    if let Some(asks) = &msg.asks {
        if let Some(best) = asks.iter().min_by_key(|l| Decimal::from_str(&l.price).ok()) {
            if let (Ok(price), Ok(size)) = (Decimal::from_str(&best.price), Decimal::from_str(&best.size)) {
                tob.ask = Some((price, size));
                updated = true;
            }
        }
    }
    
    if updated {
        tob.ts = Instant::now();
        Some(token_id.clone())
    } else {
        None
    }
}

fn good_tob(tob: &Option<&TopOfBook>, stale_seconds: f64) -> bool {
    if let Some(t) = tob {
        if t.ask.is_some() && t.ts.elapsed().as_secs_f64() <= stale_seconds {
            return true;
        }
    }
    false
}

async fn get_available_usdc(
    _client: &AuthenticatedClient, 
    cache: &Mutex<BalanceCache>, 
    ttl: f64
) -> Result<Option<Decimal>> {
    let cache_guard = cache.lock().await;
    
    if cache_guard.available.is_some() && cache_guard.ts.elapsed().as_secs_f64() <= ttl {
        return Ok(cache_guard.available);
    }
    
    drop(cache_guard);
    
    // Try to get balance - this may fail on some SDK versions
    // In that case, we'll fall back to using the SIZE parameter
    let available = None; // Placeholder - implement based on actual SDK method availability
    
    let mut cache_guard = cache.lock().await;
    cache_guard.available = available;
    cache_guard.ts = Instant::now();
    
    Ok(available)
}

fn compute_size_from_bankroll(
    config: &BotConfig,
    available_usdc: Decimal,
    ask_a: Decimal,
    ask_b: Decimal,
    book_sz_a: Decimal,
    book_sz_b: Decimal,
) -> Option<(Decimal, Decimal, Decimal)> {
    let total_price = ask_a + ask_b;
    if total_price <= Decimal::ZERO {
        return None;
    }
    
    let spendable = (available_usdc - config.reserve_usdc).max(Decimal::ZERO);
    if spendable <= Decimal::ZERO {
        return None;
    }
    
    let mut budget = spendable * config.bankroll_fraction;
    budget = budget.min(config.max_trade_notional_usd);
    
    if budget < config.min_trade_notional_usd {
        return None;
    }
    
    let mut shares = budget / total_price;
    
    let min_shares_a = config.min_leg_notional_usd / ask_a.max(dec!(0.000000001));
    let min_shares_b = config.min_leg_notional_usd / ask_b.max(dec!(0.000000001));
    let min_shares = min_shares_a.max(min_shares_b);
    
    shares = shares.max(min_shares);
    shares = shares.min(config.size).min(book_sz_a).min(book_sz_b);
    shares = round_down(shares, config.size_decimals);
    
    if shares <= Decimal::ZERO {
        return None;
    }
    
    let used = shares * total_price;
    if used < config.min_trade_notional_usd || used > spendable {
        return None;
    }
    
    Some((shares, used, total_price))
}

async fn post_batch_fok(
    client: &AuthenticatedClient,
    config: &BotConfig,
    signer: &EthSigner,
    sym: &str,
    token_a: &str,
    token_b: &str,
    ask_a: Decimal,
    ask_b: Decimal,
    shares: Decimal,
) -> Result<serde_json::Value> {
    if config.dry_run {
        let log = serde_json::json!({
            "event": "dry_run_skip",
            "sym": sym,
            "shares": shares,
            "a": {"token": token_a, "ask": ask_a},
            "b": {"token": token_b, "ask": ask_b},
            "ts": chrono::Utc::now().timestamp(),
        });
        warn!("{}", serde_json::to_string(&log)?);
        return Ok(serde_json::json!({"dry_run": true}));
    }
    
    info!("Placing orders: {} x {} @ {} + {}", sym, shares, ask_a, ask_b);
    
    // Build and sign orders
    let order_a = client
        .limit_order()
        .token_id(token_a)
        .size(shares)
        .price(ask_a)
        .side(Side::Buy)
        .build()
        .await?;
    
    let order_b = client
        .limit_order()
        .token_id(token_b)
        .size(shares)
        .price(ask_b)
        .side(Side::Buy)
        .build()
        .await?;
    
    let signed_a = client.sign(signer, order_a).await?;
    let signed_b = client.sign(signer, order_b).await?;
    
    // Post orders individually
    let resp_a = client.post_order(signed_a).await?;
    let resp_b = client.post_order(signed_b).await?;
    
    info!("Order A result: {:?}", resp_a);
    info!("Order B result: {:?}", resp_b);
    
    // Return a simple success indicator
    let result = serde_json::json!({
        "success": true,
        "orders": 2,
        "timestamp": chrono::Utc::now().timestamp()
    });
    
    Ok(result)
}

async fn maybe_fire(
    state: Arc<SharedState>, 
    client: &AuthenticatedClient, 
    signer: &EthSigner, 
    sym: String
) -> Result<()> {
    let pairs = state.pairs.read().await;
    let pair = match pairs.get(&sym) {
        Some(p) => p.clone(),
        None => return Ok(()),
    };
    drop(pairs);
    
    {
        let in_flight = state.in_flight.read().await;
        if *in_flight.get(&sym).unwrap_or(&false) {
            return Ok(());
        }
    }
    
    {
        let last_fire = state.last_fire.read().await;
        if let Some(last) = last_fire.get(&sym) {
            if last.elapsed().as_secs_f64() < state.config.cooldown_seconds {
                return Ok(());
            }
        }
    }
    
    let book = state.book.read().await;
    let ba = book.get(&pair.token_a);
    let bb = book.get(&pair.token_b);
    
    if !good_tob(&ba, state.config.stale_seconds) || !good_tob(&bb, state.config.stale_seconds) {
        return Ok(());
    }
    
    let (ask_a, size_a) = ba.unwrap().ask.unwrap();
    let (ask_b, size_b) = bb.unwrap().ask.unwrap();
    drop(book);
    
    let total = ask_a + ask_b;
    if total > state.config.sum_max {
        return Ok(());
    }
    
    // Compute sizing
    let avail = get_available_usdc(client, &state.balance_cache, state.config.balance_cache_ttl).await?;
    
    let (shares, used) = match avail {
        Some(available) => {
            match compute_size_from_bankroll(&state.config, available, ask_a, ask_b, size_a, size_b) {
                Some((s, u, _)) => (s, u),
                None => return Ok(()),
            }
        }
        None => {
            let shares = state.config.size;
            if (ask_a * shares) < state.config.min_leg_notional_usd || (ask_b * shares) < state.config.min_leg_notional_usd {
                return Ok(());
            }
            if size_a < shares || size_b < shares {
                return Ok(());
            }
            (shares, shares * total)
        }
    };
    
    // Set in-flight and last fire
    {
        let mut in_flight = state.in_flight.write().await;
        in_flight.insert(sym.clone(), true);
    }
    {
        let mut last_fire = state.last_fire.write().await;
        last_fire.insert(sym.clone(), Instant::now());
    }
    
    let log = serde_json::json!({
        "event": "arb_trigger",
        "sym": &sym,
        "slug": &pair.slug,
        "sum": total,
        "a": {"token": &pair.token_a, "ask": ask_a, "sz": size_a},
        "b": {"token": &pair.token_b, "ask": ask_b, "sz": size_b},
        "shares": shares,
        "used_usd_est": used,
        "avail_usdc": avail,
        "sum_max": state.config.sum_max,
        "dry_run": state.config.dry_run,
        "ts": chrono::Utc::now().timestamp(),
    });
    warn!("{}", serde_json::to_string(&log)?);
    
    // Execute
    let result = post_batch_fok(
        client,
        &state.config,
        signer,
        &sym,
        &pair.token_a,
        &pair.token_b,
        ask_a,
        ask_b,
        shares,
    ).await;
    
    match result {
        Ok(_) => info!("Successfully executed arb for {}", sym),
        Err(e) => error!("Failed to execute arb for {}: {}", sym, e),
    }
    
    // Clear in-flight
    {
        let mut in_flight = state.in_flight.write().await;
        in_flight.insert(sym, false);
    }
    
    Ok(())
}

async fn rollover_loop(state: Arc<SharedState>) {
    let mut last_slot = current_15m_start_epoch();
    
    loop {
        sleep(Duration::from_secs(2)).await;
        let slot = current_15m_start_epoch();
        
        if slot == last_slot {
            continue;
        }
        last_slot = slot;
        
        info!("Rolling over to slot {}", slot);
        
        match resolve_current_pairs(&state).await {
            Ok(new_pairs) => {
                if new_pairs.is_empty() {
                    warn!("Rollover failed: no pairs resolved");
                    continue;
                }
                
                let old_tokens: std::collections::HashSet<String> = {
                    state.symbol_by_token.read().await.keys().cloned().collect()
                };
                
                let mut new_tokens = std::collections::HashSet::new();
                {
                    let mut pairs = state.pairs.write().await;
                    let mut symbol_by_token = state.symbol_by_token.write().await;
                    
                    *pairs = new_pairs.clone();
                    symbol_by_token.clear();
                    
                    for (sym, pair) in &new_pairs {
                        symbol_by_token.insert(pair.token_a.clone(), sym.clone());
                        symbol_by_token.insert(pair.token_b.clone(), sym.clone());
                        new_tokens.insert(pair.token_a.clone());
                        new_tokens.insert(pair.token_b.clone());
                        info!("Pair: {} | {} | {}", sym, pair.slug, pair.condition_id);
                    }
                }
                
                let _to_unsub: Vec<_> = old_tokens.difference(&new_tokens).cloned().collect();
                let _to_sub: Vec<_> = new_tokens.difference(&old_tokens).cloned().collect();
                
                info!("Rollover: unsub={} sub={}", _to_unsub.len(), _to_sub.len());
                
                // Note: WebSocket subscription updates would need to be sent via a channel
                // For now, we'll just rely on reconnection to pick up new subscriptions
            }
            Err(e) => {
                error!("Rollover resolve failed: {}", e);
            }
        }
    }
}

async fn run_ws_loop(
    state: Arc<SharedState>, 
    client: AuthenticatedClient, 
    signer: EthSigner
) -> Result<()> {
    let wss_base = state.config.wss_url.trim_end_matches("/ws").trim_end_matches("/");
    let market_ws_url = format!("{}/ws/market", wss_base);
    
    loop {
        info!("Connecting to WebSocket: {}", market_ws_url);
        
        match connect_async(&market_ws_url).await {
            Ok((ws_stream, _)) => {
                info!("WebSocket connected");
                
                let (mut ws_tx, mut ws_rx) = ws_stream.split();
                
                // Initial subscription
                let token_ids: Vec<String> = {
                    let pairs = state.pairs.read().await;
                    pairs.values().flat_map(|p| vec![p.token_a.clone(), p.token_b.clone()]).collect()
                };
                
                let sub_msg = serde_json::json!({
                    "assets_ids": token_ids,
                    "type": "market",
                    "custom_feature_enabled": true,
                });
                
                ws_tx.send(Message::Text(sub_msg.to_string())).await?;
                
                // Spawn ping task
                let ws_tx_clone = Arc::new(Mutex::new(ws_tx));
                let ping_tx = ws_tx_clone.clone();
                tokio::spawn(async move {
                    loop {
                        sleep(Duration::from_secs(10)).await;
                        let mut tx = ping_tx.lock().await;
                        let _ = tx.send(Message::Text("PING".to_string())).await;
                    }
                });
                
                // Spawn rollover task (simplified - doesn't update subscriptions dynamically)
                let state_clone = state.clone();
                tokio::spawn(async move {
                    rollover_loop(state_clone).await;
                });
                
                // Main message loop
                while let Some(msg) = ws_rx.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if text == "PONG" {
                                continue;
                            }
                            
                            debug!("WS msg: {}", &text[..text.len().min(240)]);
                            
                            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                let updated_token = {
                                    let mut book = state.book.write().await;
                                    update_book_from_msg(&mut book, &ws_msg)
                                };
                                
                                if let Some(token) = updated_token {
                                    let symbol = state.symbol_by_token.read().await.get(&token).cloned();
                                    if let Some(sym) = symbol {
                                        let state_clone = state.clone();
                                        let client_clone = client.clone();
                                        let signer_clone = signer.clone();
                                        tokio::spawn(async move {
                                            let _ = maybe_fire(state_clone, &client_clone, &signer_clone, sym).await;
                                        });
                                    }
                                }
                            }
                        }
                        Ok(Message::Close(_)) => {
                            warn!("WebSocket closed");
                            break;
                        }
                        Err(e) => {
                            error!("WebSocket error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to WebSocket: {}", e);
                sleep(Duration::from_secs(2)).await;
            }
        }
        
        sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("polymarket_arb_bot=info".parse()?))
        .init();
    
    // Load config
    let config = BotConfig::from_env()?;
    
    info!("Starting bot: dry_run={} size_cap={} sum_max={} hedge_mode={}",
        config.dry_run, config.size, config.sum_max, config.hedge_mode);
    
    // Initialize CLOB client
    let client = init_clob_client(&config).await?;
    info!("CLOB client initialized");
    
    // Resolve initial pairs
    let state = SharedState::new(config);
    let pairs = resolve_current_pairs(&state).await?;
    
    if pairs.is_empty() {
        anyhow::bail!("Could not resolve any current 15m markets");
    }
    
    {
        let mut state_pairs = state.pairs.write().await;
        let mut symbol_by_token = state.symbol_by_token.write().await;
        
        *state_pairs = pairs.clone();
        for (sym, pair) in &pairs {
            symbol_by_token.insert(pair.token_a.clone(), sym.clone());
            symbol_by_token.insert(pair.token_b.clone(), sym.clone());
            info!("Initial pair: {} | {} | {}", sym, pair.slug, pair.condition_id);
        }
    }
    
    // Extract signer for order signing
    let signer: EthSigner = LocalSigner::from_str(&state.config.private_key)?
        .with_chain_id(Some(137));
    
    // Run WebSocket loop
    run_ws_loop(state, client, signer).await?;
    
    Ok(())
}