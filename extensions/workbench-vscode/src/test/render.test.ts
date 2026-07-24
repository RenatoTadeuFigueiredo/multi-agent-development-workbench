import assert from "node:assert/strict";
import test from "node:test";
import { renderMarkdown } from "../render";

test("renders mermaid fences through a safe presentation placeholder", () => {
  const rendered = renderMarkdown("# Plan\n```mermaid\ngraph TD; A-->B\n```");
  assert.match(rendered, /workbench-mermaid/);
  assert.match(rendered, /graph TD;/);
  assert.doesNotMatch(rendered, /<script/);
});
