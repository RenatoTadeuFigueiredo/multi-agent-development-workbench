use std::collections::BTreeSet;

use futures_util::StreamExt;
use thiserror::Error;
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{
        AuthenticationStatus, CancellationStatus, ProviderAdapter, ProviderFailure, ProviderOutput,
        ProviderPrompt,
    },
    value::NonEmptyText,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderContractReport {
    pub output_count: usize,
    pub acknowledged: bool,
    pub content_events: usize,
    pub tool_events: usize,
    pub completed: bool,
    pub cancellation: CancellationStatus,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("provider contract violation: {0}")]
pub struct ProviderContractError(String);

pub async fn verify_happy_path_contract(
    adapter: &dyn ProviderAdapter,
) -> Result<ProviderContractReport, ProviderContractError> {
    let capabilities = adapter
        .capabilities()
        .await
        .map_err(|error| core_error(&error))?;
    if capabilities.adapter_id.as_str().is_empty()
        || capabilities.adapter_version.is_empty()
        || capabilities.protocol.is_empty()
    {
        return violation("capability identity fields must not be empty");
    }
    let unique = capabilities.capabilities.iter().collect::<BTreeSet<_>>();
    if unique.len() != capabilities.capabilities.len() {
        return violation("capabilities must be unique");
    }
    let authentication = adapter
        .authentication_status()
        .await
        .map_err(|error| core_error(&error))?;
    if authentication != capabilities.authentication
        || authentication != AuthenticationStatus::Available
    {
        return violation("authentication discovery is inconsistent or unavailable");
    }

    let handle = adapter
        .start_session()
        .await
        .map_err(|failure| provider_failure(&failure))?;
    if handle.expose_to_adapter().is_empty() {
        return violation("started session handle is empty");
    }
    let resumed = adapter
        .resume_session(handle.expose_to_adapter())
        .await
        .map_err(|failure| provider_failure(&failure))?;
    if resumed.expose_to_adapter().is_empty() {
        return violation("resumed session handle is empty");
    }

    let prompt = contract_prompt();
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&resumed, prompt)
        .await
        .map_err(|failure| provider_failure(&failure))?;
    let mut report = ProviderContractReport {
        output_count: 0,
        acknowledged: false,
        content_events: 0,
        tool_events: 0,
        completed: false,
        cancellation: CancellationStatus::Unconfirmed,
    };
    while let Some(item) = stream.next().await {
        let output = item.map_err(|failure| provider_failure(&failure))?;
        report.output_count += 1;
        match output {
            ProviderOutput::Acknowledged { .. } => report.acknowledged = true,
            ProviderOutput::Content { ref event_type, .. } => {
                if event_type.is_empty() {
                    return violation("content event_type is empty");
                }
                report.content_events += 1;
            }
            ProviderOutput::Tool { ref event_type, .. } => {
                if event_type.is_empty() {
                    return violation("tool event_type is empty");
                }
                report.tool_events += 1;
            }
            ProviderOutput::Completed { ref summary } => {
                if summary.is_empty() || report.completed {
                    return violation("completion must be unique and non-empty");
                }
                report.completed = true;
            }
        }
    }
    if !report.acknowledged
        || report.content_events == 0
        || report.tool_events == 0
        || !report.completed
    {
        return violation("happy stream omitted a required normalized event");
    }
    report.cancellation = adapter
        .cancel(&resumed, attempt_id)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(report)
}

pub async fn verify_failure_contract(
    adapter: &dyn ProviderAdapter,
) -> Result<ProviderFailure, ProviderContractError> {
    let handle = adapter
        .start_session()
        .await
        .map_err(|failure| provider_failure(&failure))?;
    let prompt = contract_prompt();
    match adapter.prompt_stream(&handle, prompt).await {
        Err(failure) => validate_failure(failure),
        Ok(mut stream) => {
            while let Some(item) = stream.next().await {
                if let Err(failure) = item {
                    return validate_failure(failure);
                }
            }
            violation("failure adapter emitted no normalized failure")
        }
    }
}

fn validate_failure(failure: ProviderFailure) -> Result<ProviderFailure, ProviderContractError> {
    if failure.user_safe_message.is_empty() || failure.user_safe_message.len() > 512 {
        return violation("provider failure message must contain 1 to 512 bytes");
    }
    if !matches!(
        failure.category,
        FailureCategory::ProviderUnavailable
            | FailureCategory::ProviderTimeout
            | FailureCategory::CapabilityUnavailable
            | FailureCategory::Internal
    ) {
        return violation("provider failure uses an unrelated category");
    }
    Ok(failure)
}

fn contract_prompt() -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: "fake-runtime".to_owned(),
        content: NonEmptyText::parse("contract prompt").expect("static text"),
    }
}

fn core_error(error: &workbench_core::CoreError) -> ProviderContractError {
    ProviderContractError(format!("{:?}: {}", error.category(), error.message()))
}

fn provider_failure(failure: &ProviderFailure) -> ProviderContractError {
    ProviderContractError(format!(
        "{:?}: {}",
        failure.category, failure.user_safe_message
    ))
}

fn violation<T>(message: &str) -> Result<T, ProviderContractError> {
    Err(ProviderContractError(message.to_owned()))
}
