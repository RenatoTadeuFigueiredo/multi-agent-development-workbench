import assert from "node:assert/strict";
import test from "node:test";
import { renderEvent } from "../render";
import { SessionEvent } from "../protocol";

test("preserves Markdown and Mermaid for VS Code's native preview", () => {
  const event: SessionEvent = {
    protocol: "workbench/1", event_id: "018f47ef-9052-7b86-b31d-3f8962457776",
    session_id: "018f47ef-9052-7b86-b31d-3f8962457777", sequence: 1,
    kind: "provider_event", occurred_at: "2026-01-01T00:00:00Z",
    data: { content: "```mermaid\ngraph TD; A-->B\n```" },
  };
  assert.match(renderEvent(event), /```mermaid/);
  assert.match(renderEvent(event), /## provider_event/);
});
