import { EventEmitter } from "node:events";
import { randomBytes } from "node:crypto";
import { Socket } from "node:net";

const PROTOCOL = "workbench/1";
const DEFAULT_MAX_FRAME_BYTES = 8 * 1024 * 1024;
const MAX_SESSION_LIST_ITEMS = 100;
const UUID_V7_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const RFC3339_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const SESSION_STATES = [
  "ready",
  "running",
  "pausing",
  "paused",
  "awaiting_clarification",
  "awaiting_approval",
  "cancel_requested",
  "outcome_unknown",
  "completed",
  "failed",
  "cancelled",
  "abandoned",
  "deleting",
] as const;
type SessionState = typeof SESSION_STATES[number];

export type SessionEvent = {
  protocol: typeof PROTOCOL;
  event_id: string;
  session_id: string;
  sequence: number;
  causation_request_id?: string;
  kind: string;
  occurred_at: string;
  data: Record<string, unknown>;
};

export type SessionSummary = {
  session_id: string;
  state: SessionState;
  created_at: string;
  terminal_at?: string;
};

export type ListSessionsResult = {
  sessions: SessionSummary[];
  next_before_session_id?: string;
};

type ProtocolError = { code: string; message: string; retryable: boolean; correlation_id: string };
type ServerReply = { protocol: typeof PROTOCOL; request_id: string; ok: boolean; result?: unknown; error?: ProtocolError };
type ClientCommand = { protocol: typeof PROTOCOL; request_id: string; method: string; params: Record<string, unknown>; session_id?: string };

export class WorkbenchProtocolError extends Error {
  constructor(readonly code: string, message: string, readonly retryable = false) {
    super(message);
  }
}

export interface Transport {
  request(method: string, params: Record<string, unknown>, sessionId?: string): Promise<unknown>;
  close(): void;
  onEvent(listener: (event: SessionEvent) => void): void;
  onClosed(listener: () => void): void;
}

export type TransportConnector = (endpoint: string) => Promise<Transport>;

export async function createPersistentSession(
  endpoint: string,
  connect: TransportConnector = NdjsonTransport.connect,
): Promise<string> {
  const result = await requestFromDaemon(endpoint, connect, "session.create", { persistent: true });
  if (!isRecord(result) || typeof result.session_id !== "string") {
    throw new WorkbenchProtocolError("invalid_result", "The daemon returned an invalid session creation result.");
  }
  return result.session_id;
}

export async function listSessions(
  endpoint: string,
  limit = 50,
  beforeSessionId?: string,
  connect: TransportConnector = NdjsonTransport.connect,
): Promise<ListSessionsResult> {
  const result = await requestFromDaemon(endpoint, connect, "session.list", {
    limit,
    ...(beforeSessionId ? { before_session_id: beforeSessionId } : {}),
  });
  if (!isListSessionsResult(result)) {
    throw new WorkbenchProtocolError("invalid_result", "The daemon returned an invalid session list.");
  }
  return result;
}

export class NdjsonTransport extends EventEmitter implements Transport {
  private readonly pending = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  private buffer = "";
  private closed = false;
  private maxFrameBytes = DEFAULT_MAX_FRAME_BYTES;

  private constructor(private readonly socket: Socket) {
    super();
    socket.setEncoding("utf8");
    socket.on("data", (chunk: string) => this.consume(chunk));
    socket.on("error", () => this.fail(new WorkbenchProtocolError("transport_error", "The local Workbench connection was lost.")));
    socket.on("close", () => this.fail(new WorkbenchProtocolError("disconnected", "The local Workbench connection closed.")));
  }

  static connect(endpoint: string): Promise<NdjsonTransport> {
    return new Promise((resolve, reject) => {
      const socket = new Socket();
      socket.once("error", () => reject(new WorkbenchProtocolError("unavailable", "The local Workbench daemon is unavailable.")));
      socket.connect(endpoint, () => resolve(new NdjsonTransport(socket)));
    });
  }

  request(method: string, params: Record<string, unknown>, sessionId?: string): Promise<unknown> {
    if (this.closed) return Promise.reject(new WorkbenchProtocolError("disconnected", "The local Workbench connection is closed."));
    const request_id = uuidV7();
    const command: ClientCommand = { protocol: PROTOCOL, request_id, method, params, ...(sessionId ? { session_id: sessionId } : {}) };
    const frame = `${JSON.stringify(command)}\n`;
    if (Buffer.byteLength(frame) > this.maxFrameBytes) {
      return Promise.reject(new WorkbenchProtocolError("frame_too_large", "The request exceeds the daemon frame limit."));
    }
    return new Promise((resolve, reject) => {
      this.pending.set(request_id, { resolve, reject });
      this.socket.write(frame, (error) => {
        if (error) this.fail(new WorkbenchProtocolError("transport_error", "The local Workbench connection was lost."));
      });
    });
  }

  onEvent(listener: (event: SessionEvent) => void): void { this.on("event", listener); }
  onClosed(listener: () => void): void { this.on("closed", listener); }
  close(): void { this.fail(new WorkbenchProtocolError("closed", "The local Workbench connection was closed.")); this.socket.destroy(); }

  private consume(chunk: string): void {
    if (this.closed) return;
    this.buffer += chunk;
    let newline: number;
    while ((newline = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, newline);
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      if (Buffer.byteLength(line) > this.maxFrameBytes) {
        this.fail(new WorkbenchProtocolError("frame_too_large", "The daemon sent an oversized protocol frame."));
        return;
      }
      try { this.handle(JSON.parse(line) as unknown); }
      catch { this.fail(new WorkbenchProtocolError("invalid_frame", "The daemon sent an invalid protocol frame.")); return; }
    }
    if (Buffer.byteLength(this.buffer) > this.maxFrameBytes) this.fail(new WorkbenchProtocolError("frame_too_large", "The daemon sent an oversized protocol frame."));
  }

  private handle(value: unknown): void {
    if (!isRecord(value) || value.protocol !== PROTOCOL) throw new Error("unsupported protocol");
    if (typeof value.event_id === "string") {
      const event = value as unknown as SessionEvent;
      if (!isSessionEvent(event)) throw new Error("invalid event");
      this.emit("event", event);
      return;
    }
    if (typeof value.request_id !== "string" || typeof value.ok !== "boolean") throw new Error("invalid reply");
    const reply = value as unknown as ServerReply;
    const pending = this.pending.get(reply.request_id);
    if (!pending) return;
    this.pending.delete(reply.request_id);
    if (!reply.ok) {
      const error = reply.error;
      const message = error?.code === "unsupported_version"
        ? "The local daemon does not support this Workbench protocol. Update the daemon or extension."
        : "The daemon rejected the request.";
      pending.reject(new WorkbenchProtocolError(error?.code ?? "protocol_error", message, error?.retryable ?? false));
      return;
    }
    if (reply.result === undefined) throw new Error("missing result");
    if (isRecord(reply.result) && typeof reply.result.max_frame_bytes === "number") this.maxFrameBytes = Math.min(DEFAULT_MAX_FRAME_BYTES, reply.result.max_frame_bytes);
    pending.resolve(reply.result);
  }

  private fail(error: WorkbenchProtocolError): void {
    if (this.closed) return;
    this.closed = true;
    this.buffer = "";
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    this.socket.destroy();
    this.emit("closed");
  }
}

async function requestFromDaemon(
  endpoint: string,
  connect: TransportConnector,
  method: string,
  params: Record<string, unknown>,
): Promise<unknown> {
  const transport = await connect(endpoint);
  try {
    const initialize = await transport.request("initialize", {
      client_name: "workbench-vscode",
      client_version: "0.1.0",
      supported_protocols: [PROTOCOL],
    });
    if (!isRecord(initialize) || initialize.selected_protocol !== PROTOCOL) {
      throw new WorkbenchProtocolError("unsupported_version", "The daemon selected an incompatible protocol.");
    }
    return await transport.request(method, params);
  } finally {
    transport.close();
  }
}

export class SessionController {
  private cursor = 0;
  private readonly seen = new Set<string>();
  private transport?: Transport;
  private stopped = false;
  private generation = 0;
  private sessionId?: string;
  private listener?: (event: SessionEvent) => void;
  private reconnecting?: Promise<void>;

  constructor(
    private readonly endpoint: string,
    private readonly connect: TransportConnector = NdjsonTransport.connect,
    private readonly onNotice: (message: string) => void = () => undefined,
  ) {}

  async attach(sessionId: string, onEvent: (event: SessionEvent) => void): Promise<void> {
    this.stopped = false;
    this.sessionId = sessionId;
    this.listener = onEvent;
    const generation = ++this.generation;
    await this.connectAndAttach(generation);
  }

  prompt(text: string): Promise<unknown> { return this.command("session.prompt", { text }); }
  pause(): Promise<unknown> { return this.command("session.pause", {}); }
  resume(): Promise<unknown> { return this.command("session.resume", {}); }
  cancel(): Promise<unknown> { return this.command("session.cancel", {}); }
  redirect(instruction: string): Promise<unknown> { return this.command("session.redirect", { instruction }); }
  resolveApproval(approvalId: string, decision: "grant" | "deny"): Promise<unknown> {
    return this.command("session.approval.resolve", { approval_id: approvalId, decision });
  }

  close(): void {
    this.stopped = true;
    this.generation += 1;
    this.transport?.close();
    this.transport = undefined;
  }

  private async connectAndAttach(generation: number): Promise<void> {
    const transport = await this.connect(this.endpoint);
    try {
      if (this.stopped || generation !== this.generation) { transport.close(); return; }
      this.transport = transport;
      transport.onEvent((event) => {
        if (event.session_id !== this.sessionId || this.seen.has(event.event_id) || event.sequence <= this.cursor) return;
        this.seen.add(event.event_id);
        this.cursor = event.sequence;
        this.listener?.(event);
      });
      transport.onClosed(() => this.scheduleReconnect(generation));
      const initialize = await transport.request("initialize", {
        client_name: "workbench-vscode",
        client_version: "0.1.0",
        supported_protocols: [PROTOCOL],
      });
      if (!isRecord(initialize) || initialize.selected_protocol !== PROTOCOL) throw new WorkbenchProtocolError("unsupported_version", "The daemon selected an incompatible protocol.");
      await transport.request("session.attach", { after_sequence: this.cursor }, this.sessionId);
    } catch (error) {
      if (this.transport === transport) this.transport = undefined;
      transport.close();
      throw error;
    }
  }

  private scheduleReconnect(generation: number): void {
    if (this.reconnecting || this.stopped || generation !== this.generation) return;
    this.reconnecting = this.reconnect(generation).finally(() => { this.reconnecting = undefined; });
  }

  private async reconnect(generation: number): Promise<void> {
    this.onNotice("Workbench connection lost. Reconnecting…");
    for (let attempt = 0; attempt < 5 && !this.stopped && generation === this.generation; attempt += 1) {
      await delay(250 * 2 ** attempt);
      try { await this.connectAndAttach(generation); this.onNotice("Workbench connection restored."); return; } catch { /* retry boundedly */ }
    }
    if (!this.stopped && generation === this.generation) this.onNotice("Workbench daemon is unavailable. Use Workbench: Attach Session to retry.");
  }

  private command(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (!this.transport || !this.sessionId) return Promise.reject(new WorkbenchProtocolError("not_attached", "Attach a Workbench session first."));
    return this.transport.request(method, params, this.sessionId);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function isListSessionsResult(value: unknown): value is ListSessionsResult {
  return isRecord(value)
    && hasExactFields(value, ["sessions"], ["next_before_session_id"])
    && Array.isArray(value.sessions)
    && value.sessions.length <= MAX_SESSION_LIST_ITEMS
    && value.sessions.every(isSessionSummary)
    && (value.next_before_session_id === undefined || isUuidV7(value.next_before_session_id));
}
function isSessionSummary(value: unknown): value is SessionSummary {
  return isRecord(value)
    && hasExactFields(value, ["session_id", "state", "created_at"], ["terminal_at"])
    && isUuidV7(value.session_id)
    && isSessionState(value.state)
    && isRfc3339Timestamp(value.created_at)
    && (value.terminal_at === undefined || isRfc3339Timestamp(value.terminal_at));
}
function hasExactFields(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[],
): boolean {
  const allowed = new Set([...required, ...optional]);
  return required.every((field) => Object.hasOwn(value, field))
    && Object.keys(value).every((field) => allowed.has(field));
}
function isUuidV7(value: unknown): value is string {
  return typeof value === "string" && UUID_V7_PATTERN.test(value);
}
function isSessionState(value: unknown): value is SessionState {
  return typeof value === "string" && (SESSION_STATES as readonly string[]).includes(value);
}
function isRfc3339Timestamp(value: unknown): value is string {
  return typeof value === "string"
    && RFC3339_PATTERN.test(value)
    && !Number.isNaN(Date.parse(value));
}
function isSessionEvent(event: SessionEvent): boolean {
  return event.protocol === PROTOCOL && typeof event.event_id === "string" && typeof event.session_id === "string" && Number.isSafeInteger(event.sequence) && event.sequence > 0 && typeof event.kind === "string" && typeof event.occurred_at === "string" && isRecord(event.data);
}
function delay(milliseconds: number): Promise<void> { return new Promise((resolve) => setTimeout(resolve, milliseconds)); }

function uuidV7(): string {
  const bytes = randomBytes(16);
  const timestamp = BigInt(Date.now());
  for (let index = 5; index >= 0; index -= 1) bytes[index] = Number((timestamp >> BigInt((5 - index) * 8)) & 0xffn);
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes.toString("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
