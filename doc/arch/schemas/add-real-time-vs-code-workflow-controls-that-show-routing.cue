// DDD role: ValueObject

package schemas

// #SessionRef identifies the attached session in the presentation layer.
// DDD role: ValueObject
#SessionRef: {
	session_id: string & =~"^[0-9a-fA-F-]{36}$"
}

// #WorkflowStepRef is the active workflow step shown in the control summary.
// DDD role: ValueObject
#WorkflowStepRef: {
	workflow_id: string & !=""
	run_id:      string & !=""
	step_id:     string & !=""
	iteration:   uint & <=8
	phase:       string & !=""
}

// #ApprovalCommand is the client params for session.approval.resolve.
// DDD role: Command
#ApprovalCommand: {
	approval_id: string & =~"^[0-9a-fA-F-]{36}$"
	decision:    "grant" | "deny"
}
