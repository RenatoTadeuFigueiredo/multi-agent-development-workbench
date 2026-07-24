import * as os from "node:os";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  createPersistentSession,
  listSessions,
  SessionController,
  SessionEvent,
  SessionSummary,
  WorkbenchProtocolError,
} from "./protocol";
import { renderEvent } from "./render";
import { workspaceSocketIdForWorkspacePath } from "./workspace";

const documentUri = vscode.Uri.parse("workbench-session:/active.md");
let controller: SessionController | undefined;
let sessionId: string | undefined;
let transcript = "# Workbench Session\n";
const changed = new vscode.EventEmitter<vscode.Uri>();

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(changed);
  context.subscriptions.push(vscode.workspace.registerTextDocumentContentProvider("workbench-session", {
    onDidChange: changed.event,
    provideTextDocumentContent: () => transcript,
  }));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.newSession", newSession));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.selectSession", selectSession));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.attach", attach));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.prompt", () => prompt()));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.pause", () => control("pause")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.resume", () => control("resume")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.cancel", () => control("cancel")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.redirect", redirect));
}

async function attach(): Promise<void> {
  try {
    const targetEndpoint = await resolveCommandEndpoint();
    if (!targetEndpoint) return;
    const selected = await vscode.window.showInputBox({ prompt: "Workbench session ID", ignoreFocusOut: true });
    if (!selected) return;
    await attachSession(selected, targetEndpoint);
  } catch (error) {
    vscode.window.showErrorMessage(displayError(error));
  }
}

async function newSession(): Promise<void> {
  try {
    const targetEndpoint = await resolveCommandEndpoint();
    if (!targetEndpoint) return;
    await newSessionAt(targetEndpoint);
  } catch (error) {
    vscode.window.showErrorMessage(displayError(error));
  }
}

async function selectSession(): Promise<void> {
  try {
    const targetEndpoint = await resolveCommandEndpoint();
    if (!targetEndpoint) return;
    await selectSessionAt(targetEndpoint);
  } catch (error) {
    vscode.window.showErrorMessage(displayError(error));
  }
}

async function newSessionAt(targetEndpoint: string): Promise<void> {
  await attachSession(await createPersistentSession(targetEndpoint), targetEndpoint);
}

async function selectSessionAt(targetEndpoint: string): Promise<void> {
  const sessionItems: SessionQuickPickItem[] = [];
  const seenSessionIds = new Set<string>();
  let beforeSessionId: string | undefined;
  let nextBeforeSessionId: string | undefined;

  do {
    const page = await listSessions(targetEndpoint, 50, beforeSessionId);
    for (const session of page.sessions) {
      if (seenSessionIds.has(session.session_id)) continue;
      seenSessionIds.add(session.session_id);
      sessionItems.push({
        label: session.session_id,
        description: session.state,
        detail: sessionDetail(session),
        session,
      });
    }
    nextBeforeSessionId = page.next_before_session_id;

    if (sessionItems.length === 0) {
      const action = await vscode.window.showInformationMessage(
        "No Workbench sessions exist for this workspace.",
        "New Session",
      );
      if (action === "New Session") await newSessionAt(targetEndpoint);
      return;
    }

    const selected = await vscode.window.showQuickPick(
      [
        ...sessionItems,
        ...(nextBeforeSessionId ? [{
          label: "Load more sessions…",
          description: "Load the next page from this workspace",
          loadMore: true,
        }] : []),
      ],
      { placeHolder: "Select a Workbench session for this workspace" },
    );
    if (!selected) return;
    if (selected.session) {
      await attachSession(selected.session.session_id, targetEndpoint);
      return;
    }
    beforeSessionId = nextBeforeSessionId;
  } while (beforeSessionId);
}

type SessionQuickPickItem = vscode.QuickPickItem & {
  session?: SessionSummary;
  loadMore?: boolean;
};

async function attachSession(selected: string, targetEndpoint: string): Promise<void> {
  controller?.close();
  sessionId = selected;
  transcript = `# Workbench Session\n\nSession: \`${sessionId}\`\n`;
  changed.fire(documentUri);
  controller = new SessionController(targetEndpoint, undefined, appendNotice);
  try {
    await controller.attach(sessionId, appendEvent);
    await vscode.commands.executeCommand("markdown.showPreviewToSide", documentUri);
  } catch (error) {
    controller.close();
    controller = undefined;
    vscode.window.showErrorMessage(displayError(error));
  }
}

async function prompt(): Promise<void> {
  const text = await vscode.window.showInputBox({ prompt: "Prompt", ignoreFocusOut: true });
  if (text) await execute(() => controller!.prompt(text));
}

async function control(action: "pause" | "resume" | "cancel"): Promise<void> {
  await execute(() => controller![action]());
}

async function redirect(): Promise<void> {
  const instruction = await vscode.window.showInputBox({ prompt: "Redirect instruction", ignoreFocusOut: true });
  if (instruction) await execute(() => controller!.redirect(instruction));
}

async function execute(action: () => Promise<unknown>): Promise<void> {
  if (!controller || !sessionId) { vscode.window.showWarningMessage("Attach a Workbench session first."); return; }
  try { await action(); }
  catch (error) { vscode.window.showErrorMessage(displayError(error)); }
}

function appendEvent(event: SessionEvent): void {
  transcript += `\n${renderEvent(event)}`;
  changed.fire(documentUri);
}

function appendNotice(message: string): void {
  transcript += `\n> ${message}\n`;
  changed.fire(documentUri);
}

function sessionDetail(session: SessionSummary): string {
  const terminal = session.terminal_at ? ` · ended ${session.terminal_at}` : "";
  return `created ${session.created_at}${terminal}`;
}

async function resolveCommandEndpoint(): Promise<string | undefined> {
  const workspace = await selectWorkspace();
  if (!workspace && (vscode.workspace.workspaceFolders?.length ?? 0) > 1) return undefined;
  return endpoint(workspace);
}

async function selectWorkspace(): Promise<vscode.WorkspaceFolder | undefined> {
  const workspaces = vscode.workspace.workspaceFolders ?? [];
  if (workspaces.length <= 1) return workspaces[0];
  const selected = await vscode.window.showQuickPick(
    workspaces.map((workspace) => ({
      label: workspace.name,
      description: workspace.uri.fsPath,
      workspace,
    })),
    { placeHolder: "Select the workspace for this Workbench command" },
  );
  return selected?.workspace;
}

function endpoint(workspace: vscode.WorkspaceFolder | undefined): string {
  const configured = vscode.workspace.getConfiguration("workbench", workspace?.uri).get<string>("endpoint")?.trim();
  if (configured) return configured;
  if (!workspace) {
    throw new WorkbenchProtocolError("workspace_required", "Open a workspace or configure workbench.endpoint.");
  }
  let workspaceId: string;
  try {
    workspaceId = workspaceSocketIdForWorkspacePath(workspace.uri.fsPath);
  } catch {
    throw new WorkbenchProtocolError("workspace_unavailable", "The current workspace path is unavailable.");
  }
  if (process.platform === "linux") {
    const runtime = process.env.XDG_RUNTIME_DIR;
    if (runtime) return path.join(runtime, "workbench", `${workspaceId}.sock`);
  }
  if (process.platform === "darwin") {
    return path.join(process.env.TMPDIR ?? os.tmpdir(), `workbench-${process.getuid?.() ?? 0}`, `${workspaceId}.sock`);
  }
  throw new WorkbenchProtocolError("unsupported_platform", "Configure workbench.endpoint for this platform.");
}

function displayError(error: unknown): string {
  if (error instanceof WorkbenchProtocolError) return `Workbench: ${error.message}`;
  return "Workbench: the local daemon request failed.";
}

export function deactivate(): void { controller?.close(); }
