import * as vscode from "vscode";
import { SessionController, WorkbenchEvent } from "./protocol";
import { renderMarkdown } from "./render";

let controller: SessionController | undefined;
let sessionId: string | undefined;

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(vscode.commands.registerCommand("workbench.attach", attach));
  context.subscriptions.push(vscode.commands.registerCommand("workbench.prompt", prompt));
}

async function attach(): Promise<void> {
  sessionId = await vscode.window.showInputBox({ prompt: "Workbench session ID" });
  if (!sessionId) return;
  const endpoint = vscode.workspace.getConfiguration("workbench").get<string>("endpoint") ?? "workbench/workbench.sock";
  controller?.close();
  controller = new SessionController(endpoint);
  const output = vscode.window.createOutputChannel("Workbench Session");
  try {
    await controller.attach(sessionId, (event) => output.appendLine(formatEvent(event)));
    output.show(true);
  } catch (error) {
    vscode.window.showErrorMessage(`Workbench connection failed: ${error instanceof Error ? error.message : "unknown error"}`);
  }
}

async function prompt(): Promise<void> {
  if (!controller || !sessionId) {
    vscode.window.showWarningMessage("Attach a Workbench session first.");
    return;
  }
  const text = await vscode.window.showInputBox({ prompt: "Prompt" });
  if (text) await controller.prompt(sessionId, text);
}

function formatEvent(event: WorkbenchEvent): string {
  const payload = typeof event.payload === "string" ? event.payload : JSON.stringify(event.payload ?? {});
  return `[${event.sequence}] ${event.kind}: ${renderMarkdown(payload)}`;
}

export function deactivate(): void { controller?.close(); }
