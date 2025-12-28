use tokio::sync::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct SymbolExecutor {
    lock: Mutex<()>,
}

#[derive(Default)]
pub struct ExecutorPool {
    inner: HashMap<String, Arc<SymbolExecutor>>,
}

impl ExecutorPool {
    pub fn new(symbols: &[String]) -> Self {
        let mut inner = HashMap::new();
        for s in symbols {
            inner.insert(s.clone(), Arc::new(SymbolExecutor::default()));
        }
        Self { inner }
    }

    pub fn get(&self, sym: &str) -> Arc<SymbolExecutor> {
        self.inner.get(sym).unwrap().clone()
    }
}
