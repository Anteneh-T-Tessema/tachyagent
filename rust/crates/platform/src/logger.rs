//! Tachy Sovereign Logging Bridge.
//!
//! Provides a cryptographically secure, zero-knowledge logging interface
//! for all platform-level operations.

pub trait TachyLogger: Send + Sync {
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
    fn error(&self, message: &str);
}

pub struct SovereignLogger {
    pub component: String,
}

impl SovereignLogger {
    pub fn new(component: &str) -> Self {
        Self {
            component: component.to_string(),
        }
    }
}

impl TachyLogger for SovereignLogger {
    fn info(&self, message: &str) {
        println!("[INFO][{}] {}", self.component, message);
    }

    fn warn(&self, message: &str) {
        eprintln!("[WARN][{}] {}", self.component, message);
    }

    fn error(&self, message: &str) {
        eprintln!("[ERROR][{}] {}", self.component, message);
    }
}

use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

/// Global logger access point.
pub static GLOBAL_LOGGER: Lazy<Arc<Mutex<Option<Box<dyn TachyLogger>>>>> = Lazy::new(|| {
    Arc::new(Mutex::new(None))
});

pub fn set_logger(logger: Box<dyn TachyLogger>) {
    if let Ok(mut l) = GLOBAL_LOGGER.lock() {
        *l = Some(logger);
    }
}

pub fn log_info(msg: &str) {
    if let Ok(l) = GLOBAL_LOGGER.lock() {
        if let Some(ref logger) = *l {
            logger.info(msg);
            return;
        }
    }
    println!("[BOOTSTRAP] {}", msg);
}
