// DDD role: ValueObject

package schemas

// #AddAVersionedSessionListCommandToTheLocalWorkbench models one bounded,
// metadata-only session discovery page.
// DDD role: ValueObject
#AddAVersionedSessionListCommandToTheLocalWorkbench: {
    limit: *50 | (int & >0 & <=100)
    before_session_id?: string & !=""
    sessions: #SessionSummaries
    next_before_session_id?: string & !=""
}

#SessionSummaries: {
    values: [...#SessionSummary]
}

// #SessionSummary deliberately excludes prompts, events, configuration,
// provider output, hashes, credentials, and audit content.
#SessionSummary: {
    session_id: string & !=""
    state: #SessionState
    created_at: string & !=""
    terminal_at?: string & !=""
}

#SessionState: "ready" | "running" | "pausing" | "paused" |
    "awaiting_clarification" | "awaiting_approval" | "cancel_requested" |
    "outcome_unknown" | "completed" | "failed" | "cancelled" |
    "abandoned" | "deleting"
