export function renderMarkdown(text: string): string {
  return text.replace(/```mermaid\s*\n([\s\S]*?)```/g, (_match, diagram: string) =>
    `<div class="workbench-mermaid" data-diagram="${escapeAttribute(diagram)}"></div>`);
}

function escapeAttribute(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}
