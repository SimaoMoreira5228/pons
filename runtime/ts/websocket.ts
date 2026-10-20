import type { CallOptions } from "../client.js";
import { err, ok, type ClientFailure, type Result } from "../errors.js";
import { decodeWire, encodeWire, parseJson, stringifyCarrier, stringifyJson } from "./codec.js";
import {
  CredentialSourceError,
  cookiePathMatches,
  credentialValue,
  customAdapter,
  isCookieAuth,
  setCookieHeader,
  type CredentialValue,
} from "./credentials.js";
import type { ConfiguredClient } from "./transport.js";
import { preflightHeaders, preflightPath, preflightQuery } from "./transport.js";
import type { Validator } from "./validate.js";
import { WirePolicyError } from "./wire.js";

export type WebSocketEvent = { data?: unknown; code?: number; reason?: string };

export type WebSocketLike = {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener(type: "open" | "message" | "error" | "close", listener: (event: WebSocketEvent) => void): void;
  removeEventListener(type: "open" | "message" | "error" | "close", listener: (event: WebSocketEvent) => void): void;
};

export type WebSocketConstructor = new (
  url: string,
  protocols?: string | string[],
  options?: { headers?: Record<string, string> },
) => WebSocketLike;

export type WebSocketFailure =
  | { kind: "closed"; code: number; reason: string; cause?: unknown }
  | { kind: "send"; cause: unknown }
  | { kind: "parse"; cause: unknown }
  | { kind: "invalid_message"; cause: unknown }
  | { kind: "validation"; cause: unknown };

export type WebSocketConnection<Client, Server> = {
  send(message: Client): Result<void, WebSocketFailure>;
  messages: AsyncIterable<Result<Server, WebSocketFailure>>;
  closed: Promise<{ code: number; reason: string }>;
  close(code?: number, reason?: string): void;
  [Symbol.asyncIterator](): AsyncIterator<Result<Server, WebSocketFailure>>;
};

type HeaderDescriptor = { name: string; required: boolean; schema: unknown };
type MessageDescriptor = { schema: unknown; validate?: Validator | undefined };

export type WebSocketRequestSpec = {
  path: string | ((values: Record<string, unknown>) => string);
  pathValues?: Record<string, unknown> | undefined;
  query?: unknown;
  headers?: Record<string, unknown> | undefined;
  auth?: { alternatives?: string[][]; scopes?: Record<string, readonly string[]>; optional?: boolean } | undefined;
  inputs: {
    path: Array<{ name: string; required: boolean; schema: unknown }>;
    query?: unknown;
    headers: HeaderDescriptor[];
    bodyRequired: boolean;
  };
  client: MessageDescriptor;
  server: MessageDescriptor;
  preserveWideIntegers?: boolean | undefined;
  options?: CallOptions | undefined;
};

const OPEN = 1;
const MAX_TIMER_DELAY = 2_147_483_647;

class MessageQueue<T> implements AsyncIterable<T> {
  private buffered: T[] = [];
  private waiters: Array<(result: IteratorResult<T>) => void> = [];
  private finished = false;

  push(value: T): void {
    if (this.finished) return;
    const waiter = this.waiters.shift();
    if (waiter !== undefined) waiter({ value, done: false });
    else this.buffered.push(value);
  }

  finish(): void {
    if (this.finished) return;
    this.finished = true;
    for (const waiter of this.waiters.splice(0)) waiter({ value: undefined as never, done: true });
  }

  [Symbol.asyncIterator](): AsyncIterator<T> {
    return {
      next: (): Promise<IteratorResult<T>> => {
        const value = this.buffered.shift();
        if (value !== undefined) return Promise.resolve({ value, done: false });
        if (this.finished) return Promise.resolve({ value: undefined as never, done: true });
        return new Promise((resolve) => this.waiters.push(resolve));
      },
    };
  }
}

export async function websocket<Client, Server>(
  client: ConfiguredClient,
  spec: WebSocketRequestSpec,
): Promise<Result<WebSocketConnection<Client, Server>, ClientFailure>> {
  const callOptions = spec.options;
  const parentSignal = callOptions?.signal ?? client.signal;
  const configuredTimeout = callOptions?.timeout ?? client.timeout;
  const timeoutMs = typeof configuredTimeout === "number" && Number.isFinite(configuredTimeout)
    ? Math.min(Math.max(configuredTimeout, 0), MAX_TIMER_DELAY)
    : undefined;
  const preserveWideIntegers = spec.preserveWideIntegers === true;
  try {
    const pathValues = preflightPath(spec.pathValues, spec.inputs.path, client.opaque);
    const query = preflightQuery(spec.query, spec.inputs.query, client.opaque);
    const encodedHeaders = preflightHeaders(spec.headers, spec.inputs.headers, client.opaque);
    const baseUrl = client.baseUrl.endsWith("/") ? client.baseUrl : `${client.baseUrl}/`;
    const resolvedPath = typeof spec.path === "function" ? spec.path(pathValues) : spec.path;
    const url = new URL(resolvedPath.replace(/^\/+/, ""), baseUrl);
    if (query !== undefined && query !== null && typeof query === "object") {
      for (const [key, value] of Object.entries(query)) {
        if (value === undefined || value === null) continue;
        for (const item of Array.isArray(value) ? value : [value]) {
          url.searchParams.append(key, stringifyCarrier(item));
        }
      }
    }
    const configuredHeaders = typeof client.headers === "function" ? await client.headers() : client.headers;
    const headers = new Headers(configuredHeaders);
    if (callOptions?.headers) {
      for (const [key, value] of new Headers(callOptions.headers).entries()) headers.set(key, value);
    }
    for (const [key, value] of Object.entries(encodedHeaders)) {
      if (value !== undefined && value !== null) headers.set(key, stringifyCarrier(value));
    }
    const authFailure = await attachAuth(client, url, headers, spec.auth);
    if (authFailure !== undefined) return err(authFailure);
    const socketUrl = new URL(url.href);
    if (socketUrl.protocol === "http:") socketUrl.protocol = "ws:";
    else if (socketUrl.protocol === "https:") socketUrl.protocol = "wss:";
    else if (socketUrl.protocol !== "ws:" && socketUrl.protocol !== "wss:") {
      return err({ kind: "protocol", message: `unsupported websocket base URL protocol ${socketUrl.protocol}` });
    }
    const Constructor = client.webSocket ?? defaultWebSocketConstructor();
    if (Constructor === undefined) {
      return err({ kind: "protocol", message: "no WebSocket constructor is available in this runtime" });
    }
    const handshake: Record<string, string> = {};
    headers.forEach((value, name) => {
      handshake[name] = value;
    });
    let socket: WebSocketLike;
    try {
      socket = new Constructor(socketUrl.href, undefined, { headers: handshake });
    } catch (cause) {
      return err({ kind: "network", cause });
    }
    const connected = await waitForOpen(socket, timeoutMs, parentSignal);
    if (connected.isErr()) return err(connected.error as ClientFailure);
    return ok(connection(client, spec, socket, preserveWideIntegers, parentSignal));
  } catch (cause) {
    if (cause instanceof CredentialSourceError) {
      return err({ kind: "auth_source", scheme: cause.scheme, cause: cause.sourceCause });
    }
    if (cause instanceof WirePolicyError) return err({ kind: "protocol", message: cause.message });
    return err({ kind: "network", cause });
  }
}

function defaultWebSocketConstructor(): WebSocketConstructor | undefined {
  const candidate = (globalThis as { WebSocket?: WebSocketConstructor }).WebSocket;
  return candidate;
}

async function attachAuth(
  client: ConfiguredClient,
  url: URL,
  headers: Headers,
  auth: WebSocketRequestSpec["auth"],
): Promise<ClientFailure | undefined> {
  const groups = auth?.alternatives ?? [];
  if (groups.length === 0) return undefined;
  const authScopes = auth?.scopes ?? {};
  const values = new Map<string, CredentialValue>();
  const valueFor = async (scheme: string): Promise<CredentialValue> => {
    const cached = values.get(scheme);
    if (cached !== undefined) return cached;
    const value = await credentialValue(client, scheme, url);
    values.set(scheme, value);
    return value;
  };
  let active: string[] | undefined;
  for (const group of groups) {
    if (group.length === 0) {
      active = group;
      break;
    }
    let available = true;
    for (const scheme of group) {
      const definition = client.authSchemes[scheme];
      if (definition === undefined) {
        available = false;
        break;
      }
      if (definition.kind === "custom") {
        if (customAdapter(client, definition, scheme) === undefined) {
          available = false;
          break;
        }
        continue;
      }
      const credential = await valueFor(scheme);
      const explicitCookieState = isCookieAuth(definition) && (
        client.auth?.[scheme] !== undefined ||
        (client.authRecords[scheme]?.length ?? 0) > 0 ||
        client.authInvalidated[scheme] === true
      );
      if (!isCookieAuth(definition) && credential.origin !== undefined && credential.origin !== url.origin) {
        available = false;
        break;
      }
      if (isCookieAuth(definition) && credential.secure && url.protocol !== "https:") {
        available = false;
        break;
      }
      if (isCookieAuth(definition) && credential.path !== undefined && !cookiePathMatches(credential.path, url.pathname)) {
        available = false;
        break;
      }
      if (credential.value === undefined && (!isCookieAuth(definition) || explicitCookieState)) {
        available = false;
        break;
      }
    }
    if (available) {
      active = group;
      break;
    }
  }
  if (active === undefined) {
    if (auth?.optional) return undefined;
    return { kind: "auth", schemes: groups.flat() };
  }
  for (const scheme of active) {
    const definition = client.authSchemes[scheme];
    if (definition === undefined) continue;
    if (definition.kind === "custom") {
      const adapter = customAdapter(client, definition, scheme);
      if (adapter === undefined) continue;
      try {
        const scopes = Object.prototype.hasOwnProperty.call(authScopes, scheme) ? [...(authScopes[scheme] ?? [])] : [];
        const attachment = await adapter({ scheme, scopes, method: "GET", url, headers });
        for (const [key, value] of Object.entries(attachment.headers ?? {})) headers.set(key, value);
        for (const [key, value] of Object.entries(attachment.query ?? {})) url.searchParams.set(key, value);
        for (const [key, value] of Object.entries(attachment.cookies ?? {})) setCookieHeader(headers, key, value, scheme);
      } catch (cause) {
        return { kind: "auth_adapter", scheme, cause };
      }
      continue;
    }
    const credential = await valueFor(scheme);
    const value = credential.value;
    if (value === undefined) {
      if (!auth?.optional) return { kind: "auth", schemes: [scheme] };
      continue;
    }
    if (definition.kind === "bearer") headers.set("authorization", `Bearer ${value}`);
    else if (definition.kind === "basic") {
      const bytes = new TextEncoder().encode(value);
      const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join("");
      headers.set("authorization", `Basic ${btoa(binary)}`);
    }
    else if (definition.kind === "api_key" && definition.location === "header") headers.set(definition.name!, value);
    else if (definition.kind === "api_key" && definition.location === "query") url.searchParams.set(definition.name!, value);
    else if (isCookieAuth(definition)) setCookieHeader(headers, definition.name!, value, scheme);
  }
  return undefined;
}

function waitForOpen(
  socket: WebSocketLike,
  timeoutMs: number | undefined,
  signal: AbortSignal | undefined,
): Promise<Result<void, ClientFailure>> {
  return new Promise((resolve) => {
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const settle = (result: Result<void, ClientFailure>) => {
      if (settled) return;
      settled = true;
      socket.removeEventListener("open", onOpen);
      socket.removeEventListener("error", onError);
      socket.removeEventListener("close", onClose);
      if (timer !== undefined) clearTimeout(timer);
      if (signal !== undefined) signal.removeEventListener("abort", onAbort);
      resolve(result);
    };
    const close = () => {
      try {
        socket.close();
      } catch {
      }
    };
    const onOpen = () => settle(ok(undefined));
    const onError = (event: WebSocketEvent) => settle(err({ kind: "network", cause: event }));
    const onClose = (event: WebSocketEvent) => settle(err({
      kind: "network",
      cause: { code: event.code ?? 1006, reason: event.reason ?? "" },
    }));
    const onAbort = () => {
      close();
      settle(err({ kind: "aborted", cause: signal?.reason }));
    };
    socket.addEventListener("open", onOpen);
    socket.addEventListener("error", onError);
    socket.addEventListener("close", onClose);
    if (signal !== undefined) {
      if (signal.aborted) {
        onAbort();
        return;
      }
      signal.addEventListener("abort", onAbort, { once: true });
    }
    timer = timeoutMs === undefined
      ? undefined
      : setTimeout(() => {
        close();
        settle(err({ kind: "timeout", afterMs: timeoutMs }));
      }, timeoutMs);
  });
}

function connection<Client, Server>(
  client: ConfiguredClient,
  spec: WebSocketRequestSpec,
  socket: WebSocketLike,
  preserveWideIntegers: boolean,
  signal: AbortSignal | undefined,
): WebSocketConnection<Client, Server> {
  const messages = new MessageQueue<Result<Server, WebSocketFailure>>();
  let locallyClosed = false;
  let resolveClosed!: (value: { code: number; reason: string }) => void;
  const closed = new Promise<{ code: number; reason: string }>((resolve) => {
    resolveClosed = resolve;
  });
  const inbound = (data: unknown): Result<Server, WebSocketFailure> => {
    if (typeof data !== "string") {
      return err({ kind: "invalid_message", cause: new Error("the websocket message is not text") });
    }
    let parsed: unknown;
    try {
      parsed = parseJson(data, preserveWideIntegers);
    } catch (cause) {
      return err({ kind: "parse", cause });
    }
    let decoded: unknown;
    try {
      decoded = decodeWire(parsed, spec.server.schema, preserveWideIntegers, client.opaque);
    } catch (cause) {
      return err({ kind: "invalid_message", cause });
    }
    if (spec.server.validate !== undefined) {
      let violation: unknown;
      try {
        violation = spec.server.validate(decoded);
      } catch (cause) {
        return err({ kind: "validation", cause });
      }
      if (violation !== null) return err({ kind: "validation", cause: violation });
    }
    return ok(decoded as Server);
  };
  const onMessage = (event: WebSocketEvent) => {
    messages.push(inbound(event.data));
  };
  const onClose = (event: WebSocketEvent) => {
    const info = { code: event.code ?? 1006, reason: event.reason ?? "" };
    if (!locallyClosed) {
      messages.push(err({ kind: "closed", ...info }));
    }
    messages.finish();
    if (signal !== undefined) signal.removeEventListener("abort", onAbort);
    resolveClosed(info);
  };
  const onError = (event: WebSocketEvent) => {
    if (socket.readyState !== OPEN) {
      messages.push(err({ kind: "closed", code: 1006, reason: "socket error", cause: event }));
      messages.finish();
    }
  };
  const onAbort = () => {
    close();
  };
  const close = (code?: number, reason?: string) => {
    if (locallyClosed) return;
    locallyClosed = true;
    try {
      socket.close(code, reason);
    } catch {
    }
    messages.finish();
    if (signal !== undefined) signal.removeEventListener("abort", onAbort);
  };
  socket.addEventListener("message", onMessage);
  socket.addEventListener("close", onClose);
  socket.addEventListener("error", onError);
  if (signal !== undefined) {
    if (signal.aborted) close();
    else signal.addEventListener("abort", onAbort, { once: true });
  }
  return {
    send(message: Client) {
      if (locallyClosed || socket.readyState !== OPEN) {
        return err({ kind: "send", cause: new Error("the websocket is not open") });
      }
      try {
        socket.send(stringifyJson(encodeWire(message, spec.client.schema, client.opaque)));
        return ok(undefined);
      } catch (cause) {
        return err({ kind: "send", cause });
      }
    },
    messages,
    closed,
    close,
    [Symbol.asyncIterator]() {
      return messages[Symbol.asyncIterator]();
    },
  };
}
