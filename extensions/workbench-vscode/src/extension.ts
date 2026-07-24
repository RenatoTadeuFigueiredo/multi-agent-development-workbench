import * as os from "node:os";
import * as path from "node:path";
import * as vscode from "vscode";
import { SessionController, SessionEvent, WorkbenchProtocolError } from "./protocol";
import { renderEvent } from "./render";

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
  context.subscriptions.push(vscode.commands.registerCommand("workbench.attach", attach));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.prompt", () => prompt()));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.pause", () => control("pause")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.resume", () => control("resume")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.cancel", () => control("cancel")));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.redirect", redirect));
}

async function attach(): Promise<void> {
  const selected = await vscode.window.showInputBox({ prompt: "Workbench session ID", ignoreFocusOut: true });
  if (!selected) return;
  controller?.close();
  sessionId = selected;
  transcript = `# Workbench Session\n\nSession: \`${sessionId}\`\n`;
  changed.fire(documentUri);
  controller = new SessionController(endpoint(), undefined, appendNotice);
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

function endpoint(): string {
  const configured = vscode.workspace.getConfiguration("workbench").get<string>("endpoint")?.trim();
  if (configured) return configured;
  if (process.platform === "linux") {
    const runtime = process.env.XDG_RUNTIME_DIR;
    if (runtime) return path.join(runtime, "workbench", "workbench.sock");
  }
  if (process.platform === "darwin") return path.join(process.env.TMPDIR ?? os.tmpdir(), `workbench-${process.getuid?.() ?? 0}`, "workbench.sock");
  throw new WorkbenchProtocolError("unsupported_platform", "Configure workbench.endpoint for this platform.");
}

function displayError(error: unknown): string {
  if (error instanceof WorkbenchProtocolError) return `Workbench: ${error.message}`;
  return "Workbench: the local daemon request failed.";
}

export function deactivate(): void { controller?.close(); }
