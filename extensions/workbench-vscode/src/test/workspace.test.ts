import assert from "node:assert/strict";
import test from "node:test";
import { workspaceSocketId } from "../workspace";

test("workspaceSocketId matches the Rust cross-client SHA-256 vector", () => {
  assert.equal(
    workspaceSocketId("/workspace/example"),
    "daf6640544250076b29c16531feb382e",
  );
});
