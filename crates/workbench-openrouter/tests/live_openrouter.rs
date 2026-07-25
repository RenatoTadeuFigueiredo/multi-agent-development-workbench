//! Opt-in live `OpenRouter` smoke. Never runs in default CI.

/// Live smoke remains ignored: requires `OPENROUTER_API_KEY` and network.
#[tokio::test]
#[ignore = "live OpenRouter smoke requires credentials and network"]
async fn live_openrouter_handshake_is_opt_in() {
    // Operators may replace this body with a bounded models list probe when
    // authorizing paid or free-tier live validation outside CI defaults.
    assert!(
        std::env::var_os("OPENROUTER_API_KEY").is_some(),
        "set OPENROUTER_API_KEY to run live smoke"
    );
}
