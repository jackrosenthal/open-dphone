//! The subsystem health page.  (The hardware tests are the Diagnostics
//! app's, `apps/diagnostics`.)

use super::*;
use opendeskphone_state::{HealthStatus, Subsystem};

pub(crate) fn health() -> Box<dyn Screen> {
    Info::boxed("health", tr::HEALTH, |cx| {
        Subsystem::ALL
            .iter()
            .map(|s| {
                let status = match cx.st.health.status(*s) {
                    HealthStatus::Stopped => "stopped".to_string(),
                    HealthStatus::Starting => "starting".to_string(),
                    HealthStatus::Ok => "ok".to_string(),
                    HealthStatus::Degraded(e) => format!("degraded: {e}"),
                    HealthStatus::Failed(e) => format!("failed: {e}"),
                };
                (s.label().to_string(), status)
            })
            .collect()
    })
}
