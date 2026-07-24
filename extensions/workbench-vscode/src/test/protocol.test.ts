import assert from "node:assert/strict";
import { createServer } from "node:net";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { SessionController } from "../protocol";

test("negotiates, attaches, receives events, and sends prompt with the committed wire contract", async () => {
  const directory = await mkdtemp(join(tmpdir(), "workbench-vscode-"));
  const endpoint = join(directory, "workbench.sock");
  const commands: Array<Record<string, unknown>> = [];
  const server = createServer((socket) => {
    socket.setEncoding("utf8");
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += chunk;
      let newline: number;
      while ((newline = buffer.indexOf("\n")) >= 0) {
        const command = JSON.parse(buffer.slice(0, newline)) as Record<string, unknown>;
        buffer = buffer.slice(newline + 1);
        commands.push(command);
        const reply = (result: Record<string, unknown>) => socket.write(`${JSON.stringify({ protocol: "workbench/1", request_id: command.request_id, ok: true, result })}\n`);
        if (command.method === "initialize") reply({ selected_protocol: "workbench/1", max_frame_bytes: 8388608, max_client_queue_events: 1024, max_client_queue_bytes: 8388608 });
        if (command.method === "session.attach") {
          reply({ session_id: command.session_id, state: "ready", replay_after_sequence: 0, last_sequence: 1 });
          socket.write(`${JSON.stringify({ protocol: "workbench/1", event_id: "018f47ef-9052-7b86-b31d-3f8962457776", session_id: command.session_id, sequence: 1, kind: "provider_event", occurred_at: "2026-01-01T00:00:00Z", data: { content: "hello" } })}\n`);
        }
        if (command.method === "session.prompt") reply({ input_id: "018f47ef-9052-7b86-b31d-3f8962457778", sequence: 2 });
        if (["session.pause", "session.resume", "session.cancel", "session.redirect"].includes(String(command.method))) {
          reply({ control_id: "018f47ef-9052-7b86-b31d-3f8962457779", control: String(command.method).replace("session.", ""), state: "running" });
        }
      }
    });
  });
  await new Promise<void>((resolve) => server.listen(endpoint, resolve));
  try {
    const events: unknown[] = [];
    const controller = new SessionController(endpoint);
    await controller.attach("018f47ef-9052-7b86-b31d-3f8962457777", (event) => events.push(event));
    await controller.prompt("hello");
    await controller.pause();
    await controller.resume();
    await controller.cancel();
    await controller.redirect("continue with the revised plan");
    assert.equal(events.length, 1);
    assert.deepEqual(commands.map((command) => command.method), ["initialize", "session.attach", "session.prompt", "session.pause", "session.resume", "session.cancel", "session.redirect"]);
    assert.equal(commands[0].protocol, "workbench/1");
    assert.match(String(commands[0].request_id), /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.deepEqual(commands[0].params, { client_name: "workbench-vscode", client_version: "0.1.0", supported_protocols: ["workbench/1"] });
    assert.equal(commands[1].session_id, "018f47ef-9052-7b86-b31d-3f8962457777");
    assert.deepEqual(commands[1].params, { after_sequence: 0 });
    assert.deepEqual(commands[2].params, { text: "hello" });
    assert.deepEqual(commands[6].params, { instruction: "continue with the revised plan" });
    controller.close();
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(directory, { recursive: true, force: true });
  }
});
