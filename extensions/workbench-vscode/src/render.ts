import { SessionEvent } from "./protocol";

export type WorkflowControlSummary = {
  session_id?: string;
  session_state?: string;
  workflow_id?: string;
  run_id?: string;
  step_id?: string;
  iteration?: number;
  phase?: string;
  pending_approval_id?: string;
  last_terminal?: string;
};

/** Derive a presentation-only summary from a durable protocol event. */
export function applyEventToSummary(
  summary: WorkflowControlSummary,
  event: SessionEvent,
): WorkflowControlSummary {
  const next: WorkflowControlSummary = { ...summary };
  const data = event.data;

  switch (event.kind) {
    case "workflow_transition":
      if (typeof data.workflow_id === "string") next.workflow_id = data.workflow_id;
      if (typeof data.run_id === "string") next.run_id = data.run_id;
      if (typeof data.step_id === "string") next.step_id = data.step_id;
      if (typeof data.iteration === "number") next.iteration = data.iteration;
      if (typeof data.phase === "string") next.phase = data.phase;
      break;
    case "approval_requested":
      if (typeof data.approval_id === "string") next.pending_approval_id = data.approval_id;
      next.session_state = "awaiting_approval";
      break;
    case "approval_recorded":
      next.pending_approval_id = undefined;
      break;
    case "session_paused":
      next.session_state = "paused";
      break;
    case "session_resumed":
      next.session_state = "running";
      break;
    case "cancel_requested":
      next.session_state = "cancel_requested";
      break;
    case "session_completed":
    case "session_failed":
    case "session_cancelled":
    case "session_abandoned":
    case "outcome_unknown":
      next.session_state = event.kind.replace("session_", "");
      next.last_terminal = event.kind;
      next.pending_approval_id = undefined;
      break;
    default:
      break;
  }
  return next;
}

export function renderControlSummary(summary: WorkflowControlSummary): string {
  const lines = [
    "# Workbench Session",
    "",
    "## Control summary",
    "",
    `- Session: \`${summary.session_id ?? "unattached"}\``,
  ];
  if (summary.session_state) lines.push(`- State: \`${summary.session_state}\``);
  if (summary.workflow_id) lines.push(`- Workflow: \`${summary.workflow_id}\``);
  if (summary.run_id) lines.push(`- Run: \`${summary.run_id}\``);
  if (summary.step_id) lines.push(`- Step: \`${summary.step_id}\``);
  if (summary.iteration !== undefined) lines.push(`- Iteration: \`${summary.iteration}\``);
  if (summary.phase) lines.push(`- Phase: \`${summary.phase}\``);
  if (summary.pending_approval_id) {
    lines.push(`- Pending approval: \`${summary.pending_approval_id}\``);
  }
  if (summary.last_terminal) lines.push(`- Terminal outcome: \`${summary.last_terminal}\``);
  lines.push("", "---", "");
  return `${lines.join("\n")}\n`;
}

export function renderStatusBarText(summary: WorkflowControlSummary): string {
  if (!summary.session_id) return "Workbench";
  const shortId = summary.session_id.slice(0, 8);
  const phase = summary.phase ?? summary.session_state ?? "attached";
  const step = summary.step_id ? ` · ${summary.step_id}` : "";
  return `Workbench ${shortId} · ${phase}${step}`;
}

export function renderEvent(event: SessionEvent): string {
  switch (event.kind) {
    case "routing_planned":
      return renderRoutingPlanned(event);
    case "workflow_transition":
      return renderWorkflowTransition(event);
    case "approval_requested":
      return renderApprovalRequested(event);
    case "approval_recorded":
      return renderApprovalRecorded(event);
    case "dispatch_planned":
    case "dispatch_started":
    case "dispatch_acknowledged":
      return renderDispatch(event);
    case "session_completed":
    case "session_failed":
    case "session_cancelled":
    case "session_abandoned":
    case "outcome_unknown":
      return renderTerminal(event);
    default:
      return renderGeneric(event);
  }
}

function renderRoutingPlanned(event: SessionEvent): string {
  const plan = isRecord(event.data.plan) ? event.data.plan : event.data;
  const destination = isRecord(plan.destination) ? plan.destination : {};
  const context = isRecord(plan.context) ? plan.context : {};
  const lines = [
    `## routing_planned`,
    "",
    `- Role: \`${asString(destination.role) ?? "unknown"}\``,
    `- Model: \`${asString(destination.model_alias) ?? "unknown"}\``,
    `- Provider: \`${asString(destination.provider) ?? "unknown"}\``,
    `- Selected by: \`${asString(plan.selected_by) ?? "unknown"}\``,
    `- Risk: \`${asString(plan.risk) ?? "unknown"}\``,
    `- Permission: \`${asString(context.permission) ?? "unknown"}\``,
  ];
  if (typeof plan.intent === "string" && plan.intent) {
    lines.push(`- Intent: ${plan.intent}`);
  }
  return `${lines.join("\n")}\n`;
}

function renderWorkflowTransition(event: SessionEvent): string {
  const data = event.data;
  return [
    `## workflow_transition`,
    "",
    `- Workflow: \`${asString(data.workflow_id) ?? "unknown"}\``,
    `- Run: \`${asString(data.run_id) ?? "unknown"}\``,
    `- Step: \`${asString(data.step_id) ?? "unknown"}\``,
    `- Iteration: \`${typeof data.iteration === "number" ? data.iteration : "unknown"}\``,
    `- Phase: \`${asString(data.phase) ?? "unknown"}\``,
    `- Reason: ${asString(data.reason) ?? "—"}`,
    "",
  ].join("\n");
}

function renderApprovalRequested(event: SessionEvent): string {
  const data = event.data;
  return [
    `## approval_requested`,
    "",
    `- Approval: \`${asString(data.approval_id) ?? "unknown"}\``,
    `- Action: \`${asString(data.action) ?? "unknown"}\``,
    `- Risk: \`${asString(data.risk) ?? "unknown"}\``,
    "",
    "Use **Workbench: Resolve Approval** to grant or deny.",
    "",
  ].join("\n");
}

function renderApprovalRecorded(event: SessionEvent): string {
  const data = event.data;
  return [
    `## approval_recorded`,
    "",
    `- Approval: \`${asString(data.approval_id) ?? "unknown"}\``,
    `- Decision: \`${asString(data.decision) ?? "unknown"}\``,
    "",
  ].join("\n");
}

function renderDispatch(event: SessionEvent): string {
  const data = event.data;
  const lines = [
    `## ${event.kind}`,
    "",
    `- Attempt: \`${asString(data.attempt_id) ?? "unknown"}\``,
  ];
  if (typeof data.operation === "string") lines.push(`- Operation: \`${data.operation}\``);
  if (typeof data.effect_class === "string") lines.push(`- Effect: \`${data.effect_class}\``);
  return `${lines.join("\n")}\n`;
}

function renderTerminal(event: SessionEvent): string {
  const data = event.data;
  const detail = typeof data.reason === "string"
    ? data.reason
    : typeof data.message === "string"
      ? data.message
      : "Terminal session outcome recorded by the daemon.";
  return `## ${event.kind}\n\n${detail}\n`;
}

function renderGeneric(event: SessionEvent): string {
  const data = event.data;
  const content = data.content ?? data.message ?? data.text;
  if (typeof content === "string") {
    return `## ${event.kind}\n\n${content}\n`;
  }
  if (typeof data.diff === "string") {
    return `## ${event.kind}\n\n\`\`\`diff\n${data.diff}\n\`\`\`\n`;
  }
  if (typeof data.artifact === "string") {
    return `## ${event.kind}\n\n### Artifact\n\n\`\`\`\n${data.artifact}\n\`\`\`\n`;
  }
  if (isRecord(data.artifact) && typeof data.artifact.path === "string") {
    const body = typeof data.artifact.content === "string"
      ? data.artifact.content
      : JSON.stringify(data.artifact, null, 2);
    return `## ${event.kind}\n\n### Artifact \`${data.artifact.path}\`\n\n\`\`\`\n${body}\n\`\`\`\n`;
  }
  return `## ${event.kind}\n\n\`\`\`json\n${JSON.stringify(event.data, null, 2)}\n\`\`\`\n`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}
