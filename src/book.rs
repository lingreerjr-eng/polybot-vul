
use std::time::{Instant};

#[derive(Clone, Default)]
pub struct TopOfBook {
    pub bid: Option<(f64, f64)>,
    pub ask: Option<(f64, f64)>,
    pub ts: Instant,
}

impl TopOfBook {
    pub fn fresh(&self, max_age_secs: f64) -> bool {
        self.ts.elapsed().as_secs_f64() <= max_age_secs && self.ask.is_some()
    }
}
