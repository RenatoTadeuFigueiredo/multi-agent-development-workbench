import { Socket } from "node:net";
import { EventEmitter } from "node:events";

export type WorkbenchEvent = {
  event_id: string;
  sequence: number;
  kind: string;
  payload?: unknown;
};

export type Request = { id: string; method: string; params: Record<string, unknown> };

export interface Transport {
  request(method: string, params: Record<string, unknown>): Promise<unknown>;
  close(): void;
  onEvent(listener: (event: WorkbenchEvent) => void): void;
}

export class NdjsonTransport extends EventEmitter implements Transport {
  private readonly pending = new Map<string, (value: unknown) => void>();
  private readonly socket: Socket;
  private buffer = "";
  private counter = 0;

  private constructor(socket: Socket) {
    super();
    this.socket = socket;
    socket.setEncoding("utf8");
    socket.on("data", (chunk: string) => this.consume(chunk));
    socket.on("error", () => this.emit("error"));
    socket.on("close", () => this.emit("close"));
  }

  static connect(endpoint: string): Promise<NdjsonTransport> {
    return new Promise((resolve, reject) => {
      const socket = new Socket();
      socket.once("error", reject);
      socket.connect(endpoint, () => {
        socket.removeListener("error", reject);
        resolve(new NdjsonTransport(socket));
      });
    });
  }

  request(method: string, params: Record<string, unknown>): Promise<unknown> {
    const id = `vscode-${++this.counter}`;
    const request: Request = { id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, resolve);
      this.socket.write(`${JSON.stringify(request)}\n`, (error) => {
        if (error) {
          this.pending.delete(id);
          reject(error);
        }
      });
    });
  }

  onEvent(listener: (event: WorkbenchEvent) => void): void { this.on("event", listener); }
  close(): void { this.socket.destroy(); }

  private consume(chunk: string): void {
    this.buffer += chunk;
    let newline: number;
    while ((newline = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, newline);
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      const message = JSON.parse(line) as { id?: string; event?: WorkbenchEvent; result?: unknown };
      if (message.id && this.pending.has(message.id)) {
        this.pending.get(message.id)!(message.result);
        this.pending.delete(message.id);
      } else if (message.event) {
        this.emit("event", message.event);
      }
    }
  }
}

export class SessionController {
  private cursor = 0;
  private readonly seen = new Set<string>();
  private transport?: Transport;
  private reconnecting = false;

  constructor(private readonly endpoint: string) {}

  async attach(sessionId: string, onEvent: (event: WorkbenchEvent) => void): Promise<void> {
    this.transport = await NdjsonTransport.connect(this.endpoint);
    this.transport.onEvent((event) => {
      if (this.seen.has(event.event_id) || event.sequence <= this.cursor) return;
      this.seen.add(event.event_id);
      this.cursor = event.sequence;
      onEvent(event);
    });
    const reconnectable = this.transport as NdjsonTransport;
    reconnectable.on("close", () => {
      if (this.reconnecting) return;
      this.reconnecting = true;
      setTimeout(() => {
        this.reconnecting = false;
        void this.attach(sessionId, onEvent).catch(() => undefined);
      }, 500);
    });
    await this.transport.request("initialize", { protocol_version: 1 });
    await this.transport.request("session.attach", { session_id: sessionId, after: this.cursor });
  }

  prompt(sessionId: string, text: string): Promise<unknown> {
    if (!this.transport) throw new Error("Workbench session is not attached");
    return this.transport.request("session.prompt", { session_id: sessionId, text });
  }

  close(): void { this.transport?.close(); }
}
