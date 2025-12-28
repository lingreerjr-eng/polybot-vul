#[derive(Debug)]
pub struct SizeResult {
    pub shares: f64,
    pub used_usd: f64,
    pub total_price: f64,
}

pub fn compute_size(
    available_usdc: f64,
    ask_a: f64,
    ask_b: f64,
    book_sz_a: f64,
    book_sz_b: f64,
    bankroll_frac: f64,
    reserve: f64,
    min_leg_usd: f64,
    min_total_usd: f64,
    max_total_usd: f64,
    size_cap: f64,
    decimals: u32,
) -> Option<SizeResult> {
    let total_price = ask_a + ask_b;
    if total_price <= 0.0 {
        return None;
    }

    let spendable = (available_usdc - reserve).max(0.0);
    if spendable <= 0.0 {
        return None;
    }

    let mut budget = spendable * bankroll_frac;
    budget = budget.min(max_total_usd);

    if budget < min_total_usd {
        return None;
    }

    let mut shares = budget / total_price;

    let min_shares_a = min_leg_usd / ask_a.max(1e-9);
    let min_shares_b = min_leg_usd / ask_b.max(1e-9);
    shares = shares.max(min_shares_a.max(min_shares_b));

    shares = shares.min(size_cap);
    shares = shares.min(book_sz_a).min(book_sz_b);

    let factor = 10_f64.powi(decimals as i32);
    shares = (shares * factor).floor() / factor;

    if shares <= 0.0 {
        return None;
    }

    let used = shares * total_price;
    if used < min_total_usd || used > spendable {
        return None;
    }

    Some(SizeResult {
        shares,
        used_usd: used,
        total_price,
    })
}

