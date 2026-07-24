use std::sync::{Arc, Mutex};

use workbench_core::ports::Telemetry;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TelemetryRecord {
    Route {
        selected_rule: &'static str,
        outcome: &'static str,
    },
    Attempt {
        outcome: &'static str,
    },
}

#[derive(Clone, Debug, Default)]
pub struct TelemetrySink {
    records: Arc<Mutex<Vec<TelemetryRecord>>>,
}

impl TelemetrySink {
    #[must_use]
    pub fn records(&self) -> Vec<TelemetryRecord> {
        self.records
            .lock()
            .expect("telemetry mutex poisoned")
            .clone()
    }

    pub fn clear(&self) {
        self.records
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
    }
}

impl Telemetry for TelemetrySink {
    fn record_route(&self, selected_rule: &'static str, outcome: &'static str) {
        self.records
            .lock()
            .expect("telemetry mutex poisoned")
            .push(TelemetryRecord::Route {
                selected_rule,
                outcome,
            });
    }

    fn record_attempt(&self, outcome: &'static str) {
        self.records
            .lock()
            .expect("telemetry mutex poisoned")
            .push(TelemetryRecord::Attempt { outcome });
    }
}
