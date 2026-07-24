import { SessionEvent } from "./protocol";

export function renderEvent(event: SessionEvent): string {
  const data = event.data.content ?? event.data.message ?? event.data.text;
  const body = typeof data === "string" ? data : `\`\`\`json\n${JSON.stringify(event.data, null, 2)}\n\`\`\``;
  return `## ${event.kind}\n\n${body}\n`;
}
