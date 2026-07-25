//! `OpenRouter` Chat Completions adapter with credential and cost controls.
//!
//! Protocol identity: `openrouter-chat-completions/1`.

#![forbid(unsafe_code)]

mod adapter;
mod budget;
mod credential;
mod error;
mod platform_secret;
mod protocol;
mod transport;

pub use adapter::{OpenRouterConnect, OpenRouterProviderAdapter};
pub use budget::{BudgetDecision, CostPolicyConfig, SessionCostLedger};
pub use credential::{MemorySecretSource, SecretSource};
pub use error::{OpenRouterError, OpenRouterErrorKind};
pub use platform_secret::PlatformSecretSource;
pub use transport::{FakeHttpMode, FakeOpenRouterTransport, OpenRouterTransport};

/// Maximum encoded response body size, excluding framing overhead.
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Locked protocol identity implemented by this adapter.
pub const OPENROUTER_CHAT_COMPLETIONS_PROTOCOL: &str = "openrouter-chat-completions/1";

/// Default public `OpenRouter` API base URL.
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Offline fake base URL (`fake://openrouter`) accepted by configuration validation.
pub const FAKE_BASE_URL: &str = "fake://openrouter";
