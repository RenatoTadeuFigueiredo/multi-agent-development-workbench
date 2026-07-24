use std::sync::{Arc, Mutex};

use time::{Duration, OffsetDateTime};
use workbench_core::ports::Clock;

/// Cloneable deterministic clock advanced only by the test.
#[derive(Clone, Debug)]
pub struct FakeClock {
    now: Arc<Mutex<OffsetDateTime>>,
}

impl FakeClock {
    #[must_use]
    pub fn new(now: OffsetDateTime) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    pub fn advance(&self, duration: Duration) -> OffsetDateTime {
        let mut now = self.now.lock().expect("fake clock mutex poisoned");
        *now += duration;
        *now
    }

    pub fn set(&self, value: OffsetDateTime) {
        *self.now.lock().expect("fake clock mutex poisoned") = value;
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new(OffsetDateTime::UNIX_EPOCH)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().expect("fake clock mutex poisoned")
    }
}
