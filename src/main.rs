mod config;
mod book;
mod execution;

use config::Config;
use rs_clob_client::client::ClobClient;
use dotenvy::dotenv;

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

