import { createHash } from "node:crypto";
import { realpathSync } from "node:fs";

const WORKSPACE_ID_DOMAIN = "workbench-workspace-id-v1\0";

/**
 * Derives the same workspace identifier as the Rust daemon from a canonical
 * workspace path.
 */
export function workspaceSocketId(canonicalPath: string): string {
  return createHash("sha256")
    .update(WORKSPACE_ID_DOMAIN)
    .update(canonicalPath)
    .digest("hex")
    .slice(0, 32);
}

/**
 * Resolves aliases before deriving the workspace-specific daemon socket ID.
 */
export function workspaceSocketIdForWorkspacePath(workspacePath: string): string {
  return workspaceSocketId(realpathSync(workspacePath));
}
