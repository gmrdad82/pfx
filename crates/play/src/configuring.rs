use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Default)]
pub struct Configuring(Arc<AtomicBool>);

impl Configuring {
    pub fn pending(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn set_pending(&self, pending: bool) {
        self.0.store(pending, Ordering::Release);
    }
}
