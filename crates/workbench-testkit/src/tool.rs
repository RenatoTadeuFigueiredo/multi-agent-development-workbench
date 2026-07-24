use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use serde_json::Value;
use workbench_core::{CoreError, FailureCategory};

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub operation: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutcome {
    pub content: Value,
}

#[derive(Clone)]
pub struct FakeTool {
    outcome: Result<ToolOutcome, CoreError>,
    calls: Arc<Mutex<Vec<ToolCall>>>,
    call_count: Arc<AtomicU64>,
}

impl FakeTool {
    #[must_use]
    pub fn succeeding(content: Value) -> Self {
        Self {
            outcome: Ok(ToolOutcome { content }),
            calls: Arc::default(),
            call_count: Arc::default(),
        }
    }

    #[must_use]
    pub fn failing(category: FailureCategory, message: impl Into<String>) -> Self {
        Self {
            outcome: Err(CoreError::new(category, message)),
            calls: Arc::default(),
            call_count: Arc::default(),
        }
    }

    pub fn execute(
        &self,
        operation: impl Into<String>,
        arguments: Value,
    ) -> Result<ToolOutcome, CoreError> {
        let operation = operation.into();
        if operation.is_empty() || !arguments.is_object() {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "fake tool requires an operation and object arguments",
            ));
        }
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.calls
            .lock()
            .expect("fake tool mutex poisoned")
            .push(ToolCall {
                operation,
                arguments,
            });
        self.outcome.clone()
    }

    #[must_use]
    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn calls(&self) -> Vec<ToolCall> {
        self.calls.lock().expect("fake tool mutex poisoned").clone()
    }
}
