
use std::env;

#[derive(Clone)]
pub struct Config {
    pub clob_host: String,
    pub wss_url: String,
    pub gamma_host: String,

    pub private_key: String,
    pub funder: Option<String>,
    pub signature_type: u8,

    pub dry_run: bool,
    pub sum_max: f64,
    pub size_cap: f64,

    pub bankroll_fraction: f64,
    pub reserve_usdc: f64,

    pub max_daily_loss: f64,
    pub max_inventory_per_token: f64,
}

impl Config {
    pub fn load() -> Self {
        Self {
            clob_host: env::var("CLOB_HOST")
                .unwrap_or("https://clob.polymarket.com".into()),
            wss_url: env::var("WSS_URL")
                .unwrap_or("wss://ws-subscriptions-clob.polymarket.com/ws/market".into()),
            gamma_host: env::var("GAMMA_HOST")
                .unwrap_or("https://gamma-api.polymarket.com".into()),

            private_key: env::var("PRIVATE_KEY").expect("PRIVATE_KEY missing"),
            funder: env::var("FUNDER_ADDRESS").ok(),
            signature_type: env::var("SIGNATURE_TYPE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),

            dry_run: env::var("DRY_RUN").unwrap_or("true".into()) == "true",
            sum_max: env::var("SUM_MAX").unwrap_or("0.97".into()).parse().unwrap(),
            size_cap: env::var("SIZE").unwrap_or("5".into()).parse().unwrap(),

            bankroll_fraction: env::var("BANKROLL_FRACTION")
                .unwrap_or("0.25".into())
                .parse()
                .unwrap(),
            reserve_usdc: env::var("RESERVE_USDC")
                .unwrap_or("2.0".into())
                .parse()
                .unwrap(),

            max_daily_loss: env::var("MAX_DAILY_LOSS").unwrap_or("25".into()).parse().unwrap(),
            max_inventory_per_token: env::var("MAX_INVENTORY").unwrap_or("50".into()).parse().unwrap(),
        }
    }
}
