import assert from "node:assert/strict";
import test from "node:test";
import {
  applyEventToSummary,
  renderControlSummary,
  renderEvent,
  renderStatusBarText,
  WorkflowControlSummary,
} from "../render";
import { SessionEvent } from "../protocol";

function baseEvent(kind: string, data: Record<string, unknown>, sequence = 1): SessionEvent {
  return {
    protocol: "workbench/1",
    event_id: "018f47ef-9052-7b86-b31d-3f8962457776",
    session_id: "018f47ef-9052-7b86-b31d-3f8962457777",
    sequence,
    kind,
    occurred_at: "2026-01-01T00:00:00Z",
    data,
  };
}

test("preserves Markdown and Mermaid for VS Code's native preview", () => {
  const event = baseEvent("provider_event", { content: "```mermaid\ngraph TD; A-->B\n```" });
  assert.match(renderEvent(event), /```mermaid/);
  assert.match(renderEvent(event), /## provider_event/);
});

test("renders routing plans with destination role model and provider", () => {
  const event = baseEvent("routing_planned", {
    plan: {
      intent: "implement step",
      destination: { role: "coder", model_alias: "codex-default", provider: "codex" },
      context: { permission: "approval-required", tools: [], data_sources: [] },
      risk: "medium",
      selected_by: "workflow",
      confidence: 1,
    },
  });
  const rendered = renderEvent(event);
  assert.match(rendered, /## routing_planned/);
  assert.match(rendered, /coder/);
  assert.match(rendered, /codex-default/);
  assert.match(rendered, /workflow/);
  assert.match(rendered, /approval-required/);
});

test("renders workflow transitions with step iteration and phase", () => {
  const event = baseEvent("workflow_transition", {
    workflow_id: "primary",
    run_id: "run-1",
    step_id: "review",
    iteration: 2,
    phase: "awaiting_human",
    reason: "max_iterations",
  });
  const rendered = renderEvent(event);
  assert.match(rendered, /## workflow_transition/);
  assert.match(rendered, /review/);
  assert.match(rendered, /awaiting_human/);
  assert.match(rendered, /max_iterations/);
});

test("renders approval requests and simple diffs", () => {
  const approval = baseEvent("approval_requested", {
    approval_id: "018f47ef-9052-7b86-b31d-3f8962457780",
    action: "write_file",
    risk: "high",
  });
  assert.match(renderEvent(approval), /approval_requested/);
  assert.match(renderEvent(approval), /write_file/);
  const diff = baseEvent("provider_event", { diff: "--- a\n+++ b\n" });
  assert.match(renderEvent(diff), /```diff/);
});

test("control summary tracks workflow and pending approval from events", () => {
  let summary: WorkflowControlSummary = { session_id: "018f47ef-9052-7b86-b31d-3f8962457777" };
  summary = applyEventToSummary(summary, baseEvent("workflow_transition", {
    workflow_id: "primary",
    run_id: "run-1",
    step_id: "spec",
    iteration: 1,
    phase: "running",
    reason: "advance",
  }));
  summary = applyEventToSummary(summary, baseEvent("approval_requested", {
    approval_id: "018f47ef-9052-7b86-b31d-3f8962457780",
    action: "write_file",
    risk: "high",
  }, 2));
  const doc = renderControlSummary(summary);
  assert.match(doc, /Control summary/);
  assert.match(doc, /primary/);
  assert.match(doc, /spec/);
  assert.match(doc, /018f47ef-9052-7b86-b31d-3f8962457780/);
  assert.match(renderStatusBarText(summary), /Workbench 018f47ef/);
  assert.match(renderStatusBarText(summary), /running/);
  assert.match(renderStatusBarText(summary), /spec/);
});
