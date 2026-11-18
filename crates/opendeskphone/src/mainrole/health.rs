//! The "healthy" gate reported to the init role.
//!
//! Subsystems that must be up before the phone counts as healthy (network,
//! UI) call [`Health::require`] when they start and [`Health::satisfy`] when
//! they are working.  The main role beats "healthy" to the init role once
//! every requirement is satisfied; that confirms a trial boot (section 12).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct Health {
    conditions: Arc<Mutex<BTreeMap<String, bool>>>,
}

impl Health {
    pub fn new() -> Self {
        Health::default()
    }

    /// Declare a condition that must hold.  Call before start-up finishes.
    pub fn require(&self, name: &str) {
        if let Ok(mut c) = self.conditions.lock() {
            c.entry(name.to_string()).or_insert(false);
        }
    }

    pub fn satisfy(&self, name: &str) {
        self.set(name, true);
    }

    /// Mark a condition as no longer holding.  The init role does not undo a
    /// confirmed trial; this only shows in `dphone health`.
    pub fn fail(&self, name: &str) {
        self.set(name, false);
    }

    fn set(&self, name: &str, value: bool) {
        if let Ok(mut c) = self.conditions.lock() {
            c.insert(name.to_string(), value);
        }
    }

    /// True when at least one condition exists and all hold.
    pub fn is_healthy(&self) -> bool {
        match self.conditions.lock() {
            Ok(c) => !c.is_empty() && c.values().all(|v| *v),
            Err(_) => false,
        }
    }

    pub fn status(&self) -> Vec<(String, bool)> {
        match self.conditions.lock() {
            Ok(c) => c.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate() {
        let h = Health::new();
        assert!(!h.is_healthy());
        h.require("core");
        h.require("net");
        h.satisfy("core");
        assert!(!h.is_healthy());
        h.satisfy("net");
        assert!(h.is_healthy());
        h.require("net");
        assert!(h.is_healthy(), "require does not reset a satisfied condition");
        h.fail("net");
        assert!(!h.is_healthy());
    }
}
