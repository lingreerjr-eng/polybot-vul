mod config;
mod book;
mod execution;

use config::Config;
use rs_clob_client::client::ClobClient;
use dotenvy::dotenv;

use executor::ExecutorPool;
use risk::{RiskLimits, RiskState};

let mut risk = RiskState::default();
let limits = RiskLimits {
    max_daily_loss: cfg.max_daily_loss,
    max_inventory_per_token: cfg.max_inventory_per_token,
};

let symbols: Vec<String> = pairs.iter().map(|p| p.symbol.clone()).collect();
let executors = ExecutorPool::new(&symbols);

use std::collections::HashMap;
use rs_clob_client::client::ClobClient;

use crate::book::TopOfBook;
use crate::gamma::MarketPair;
use crate::execution::post_fok_pair_presigned;
use crate::risk::{RiskState, RiskLimits};
use crate::config::Config;

async fn maybe_fire(
    client: &ClobClient,
    sym: &str,
    pair: &MarketPair,
    books: &HashMap<String, TopOfBook>,
    risk: &mut RiskState,
    limits: &RiskLimits,
    cfg: &Config,
) -> anyhow::Result<()> {
    // ---- book presence ----
    let tob_a = match books.get(&pair.token_a) {
        Some(v) => v,
        None => return Ok(()),
    };
    let tob_b = match books.get(&pair.token_b) {
        Some(v) => v,
        None => return Ok(()),
    };

    // ---- freshness ----
    if !tob_a.fresh(cfg.stale_seconds) || !tob_b.fresh(cfg.stale_seconds) {
        return Ok(());
    }

    let (ask_a, size_a) = tob_a.ask.unwrap();
    let (ask_b, size_b) = tob_b.ask.unwrap();

    let total = ask_a + ask_b;
    if total > cfg.sum_max {
        return Ok(());
    }

    let shares = cfg.size_cap.min(size_a).min(size_b);
    if shares <= 0.0 {
        return Ok(());
    }

    // 🔴 THIS IS WHERE THE RISK CHECK GOES
    if !risk.can_trade(&limits, &pair.token_a, &pair.token_b) {
        log::warn!("RISK BLOCK | {}", sym);
        return Ok(());
    }

    // ---- execute ----
    let resp = post_fok_pair_presigned(
        client,
        &pair.token_a,
        &pair.token_b,
        ask_a,
        ask_b,
        shares,
        cfg.dry_run,
    ).await?;

    // 🟢 THIS IS WHERE INVENTORY IS UPDATED
    if !cfg.dry_run && !resp.is_empty() {
        risk.apply_fill(&pair.token_a, shares);
        risk.apply_fill(&pair.token_b, shares);
    }

    Ok(())
}


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv();
    env_logger::init();

    let cfg = Config::load();

    let client = ClobClient::new(
        &cfg.clob_host,
        &cfg.private_key,
        137,
        cfg.signature_type,
        cfg.funder.clone(),
    ).await?;

    log::info!("arb bot booted, dry_run={}", cfg.dry_run);

    // Gamma resolution + WS loop omitted for brevity,
    // identical logic to Python version, just typed.

    Ok(())
}

