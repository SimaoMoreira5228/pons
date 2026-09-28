fn render_core() -> &'static str {
    r#"class ResponseHeaderDecodeError extends Error {}
class WirePolicyError extends Error {}
class CredentialSourceError extends Error {
  constructor(readonly scheme: string, readonly sourceCause: unknown) {
    super(`credential source failed for ${scheme}`);
  }
}

type CredentialLocator =
  | { from: "body"; pointer: string }
  | { from: "header"; name: string }
  | { from: "set_cookie"; name: string };
type CredentialProvision = { scheme: string; value: CredentialLocator; expiresAt?: CredentialLocator };
type ConfiguredClient = ClientOptions & { authValues: Record<string, string>; authExpirations: Record<string, number> };

async function request<T, E>(
  client: ConfiguredClient,
  method: string,
  path: string,
  query: unknown,
  requestHeaders: Record<string, unknown> | undefined,
  body: unknown,
  content: string | undefined,
  mediaType: string | undefined,
  authGroups: string[][],
  authScopes: Record<string, readonly string[]>,
  bodySchema: unknown,
  responses: Array<{ status: number | string; content: string; mediaType?: string; schema?: unknown; stream?: "sse" | "ndjson"; item?: unknown; events?: Array<{ event: string; schema: unknown }>; headers?: Array<{ name: string; required: boolean; schema: unknown }> }>,
  errors: Array<{ status: number | string; content: string; mediaType?: string; schema?: unknown; retryable?: boolean; events?: Array<{ event: string; schema: unknown }>; headers?: Array<{ name: string; required: boolean; schema: unknown }> }>,
  authOptional: boolean,
  preserveWideIntegers: boolean,
  provides: CredentialProvision | undefined,
  callOptions: CallOptions | undefined,
  idempotent: boolean,
): Promise<Result<T, E | ClientFailure>> {
  const timeoutMs = callOptions?.timeout ?? client.timeout;
  const parentSignal = callOptions?.signal ?? client.signal;
  const retry = callOptions?.retry === false ? undefined : { ...client.retry, ...(callOptions?.retry ?? {}) };
  const timeoutController = timeoutMs === undefined ? undefined : new AbortController();
  let timedOut = false;
  const timeoutId = timeoutController === undefined
    ? undefined
    : setTimeout(() => {
        timedOut = true;
        timeoutController.abort();
      }, timeoutMs);
  const abortParent = () => timeoutController?.abort();
  if (timeoutController && parentSignal) {
    if (parentSignal.aborted) timeoutController.abort();
    else parentSignal.addEventListener("abort", abortParent, { once: true });
  }
  const signal = timeoutController?.signal ?? parentSignal;
  try {
    const baseUrl = client.baseUrl.endsWith("/") ? client.baseUrl : `${client.baseUrl}/`;
    const url = new URL(path.replace(/^\/+/, ""), baseUrl);
    if (query && typeof query === "object") {
      for (const [key, value] of Object.entries(query)) {
        if (value === undefined || value === null) continue;
        for (const item of Array.isArray(value) ? value : [value]) {
          url.searchParams.append(key, String(item));
        }
      }
    }
    const configuredHeaders = typeof client.headers === "function" ? await client.headers() : client.headers;
    const headers = new Headers(configuredHeaders);
    if (callOptions?.headers) {
      for (const [key, value] of new Headers(callOptions.headers).entries()) headers.set(key, value);
    }
    for (const [key, value] of Object.entries(requestHeaders ?? {})) {
      if (value !== undefined && value !== null) headers.set(key, String(value));
    }
    let credentials: RequestCredentials | undefined;
    const activeGroup = authGroups.find((group) => group.every((scheme) => {
      const definition = authSchemes[scheme];
      return definition !== undefined && (
        (definition.kind === "custom" && client.authAdapters?.[scheme] !== undefined) ||
        (definition.kind !== "custom" && (client.authValues[scheme] !== undefined || client.auth?.[scheme] !== undefined || definition.kind === "cookie"))
      );
    }));
    if (authGroups.length && !activeGroup && !authOptional) {
      return err({ kind: "auth", schemes: authGroups.flat() });
    }
    for (const scheme of activeGroup ?? []) {
      const definition = authSchemes[scheme];
      if (definition === undefined) continue;
      if (definition.kind === "custom") {
        const adapter = client.authAdapters?.[scheme];
        if (adapter === undefined) continue;
        try {
          const attachment = await adapter({ scheme, scopes: authScopes[scheme] ?? [], method, url, headers });
          for (const [key, value] of Object.entries(attachment.headers ?? {})) headers.set(key, value);
          for (const [key, value] of Object.entries(attachment.query ?? {})) url.searchParams.set(key, value);
          for (const [key, value] of Object.entries(attachment.cookies ?? {})) headers.append("cookie", `${key}=${value}`);
        } catch (cause) {
          return err({ kind: "auth_adapter", scheme, cause });
        }
        continue;
      }
      if (definition.kind === "cookie") {
        credentials = "include";
        continue;
      }
      const value = await credentialValue(client, scheme);
      if (value === undefined) {
        if (!authOptional) return err({ kind: "auth", schemes: [scheme] });
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
      else if (definition.kind === "api_key" && definition.location === "cookie") headers.set("cookie", `${definition.name}=${value}`);
    }
    const init: RequestInit = { method, headers, credentials, signal };
    if (body !== undefined) {
      const encodedBody = ["json", "problem_json", "form", "multipart"].includes(content ?? "")
        ? encodeWire(body, bodySchema)
        : body;
      if (content === "json" || content === "problem_json") {
        headers.set("content-type", mediaType ?? (content === "problem_json" ? "application/problem+json" : "application/json"));
        init.body = stringifyJson(encodedBody);
      } else if (content === "form") {
        const form = new URLSearchParams();
        for (const [key, value] of Object.entries(encodeWire(body, bodySchema) as Record<string, unknown>)) {
          if (value !== undefined && value !== null) form.set(key, String(value));
        }
        headers.set("content-type", mediaType ?? "application/x-www-form-urlencoded");
        init.body = form;
      } else if (content === "multipart") {
        const form = new FormData();
        for (const [key, value] of Object.entries(encodedBody as Record<string, unknown>)) {
          if (value !== undefined && value !== null) form.set(key, value as string | Blob);
        }
        init.body = form;
      } else if (content === "text") {
        headers.set("content-type", mediaType ?? "text/plain; charset=utf-8");
        init.body = String(body);
      } else {
        if (mediaType) headers.set("content-type", mediaType);
        init.body = body as BodyInit;
      }
    }
    if (client.interceptors?.request) {
      try {
        await client.interceptors.request({ url, init });
      } catch (cause) {
        return err({ kind: "interceptor", phase: "request", cause });
      }
    }
    const response = await fetchWithRetry(client.fetch ?? fetch, url, init, retry, idempotent, errors, signal);
    if (client.interceptors?.response) {
      try {
        await client.interceptors.response({ response, url, init });
      } catch (cause) {
        return err({ kind: "interceptor", phase: "response", cause });
      }
    }
    const responseContent = response.headers.get("content-type") ?? "";
    const declaredSuccess = findSuccess(responses, response.status);
    const declaredError = declaredSuccess === undefined ? findDeclared(errors, response.status) : undefined;
    if (declaredSuccess === undefined && declaredError === undefined && response.status >= 200 && response.status < 300) {
      return err({ kind: "protocol", message: `undeclared success response ${response.status}` });
    }
    const responseSchema = declaredSuccess ?? declaredError;
    const missingHeader = responseSchema?.headers?.find((header) => header.required && !response.headers.has(header.name));
    if (missingHeader) {
      return err({ kind: "protocol", message: `missing required response header ${missingHeader.name}` });
    }
    const responseKind = responseSchema?.content;
    const expectedContentTypes: Record<string, string> = {
      json: "application/json",
      problem_json: "application/problem+json",
      form: "application/x-www-form-urlencoded",
      multipart: "multipart/form-data",
      text: "text/plain",
      bytes: "application/octet-stream",
    };
    const expectedContentType = responseSchema?.mediaType ?? (declaredSuccess?.content === "stream"
      ? declaredSuccess.stream === "ndjson" ? "application/x-ndjson" : "text/event-stream"
      : responseKind ? expectedContentTypes[responseKind] : undefined);
    const actualContentType = responseContent.split(";", 1)[0].trim().toLowerCase();
    if (expectedContentType !== undefined && actualContentType !== mediaTypeBase(expectedContentType)) {
      return err({ kind: "protocol", message: `expected ${expectedContentType}, received ${responseContent || "no content type"}` });
    }
    if (declaredSuccess?.content === "stream") {
      if (!response.body) return err({ kind: "protocol", message: "the event stream has no response body" });
      await captureCredential(client, response, provides, undefined, false);
      if (declaredSuccess.stream === "ndjson") {
        return ok(withSuccessStatus(readNdjson(response.body, declaredSuccess.item, preserveWideIntegers), response, declaredSuccess, preserveWideIntegers) as T);
      }
      return ok(withSuccessStatus(readSse(response.body, declaredSuccess.events ?? [], preserveWideIntegers), response, declaredSuccess, preserveWideIntegers) as T);
    }
    const jsonResponse = responseKind === "json" || responseKind === "problem_json" ||
      (!responseKind && responseContent.includes("json"));
    const textResponse = responseKind === "text" ||
      (!responseKind && responseContent.startsWith("text/"));
    let rawValue: unknown;
    if (response.status === 204 || response.status === 304 || responseKind === "empty") {
      rawValue = undefined;
    } else if (jsonResponse) {
      const text = await response.text();
      try {
        rawValue = parseJson(text, preserveWideIntegers);
      } catch {
        return err({ kind: "protocol", message: "invalid JSON response" });
      }
    } else if (textResponse) {
      rawValue = await response.text();
    } else {
      rawValue = new Uint8Array(await response.arrayBuffer());
    }
    const value = jsonResponse && responseSchema?.schema
      ? decodeWire(rawValue, responseSchema.schema)
      : rawValue;
    if (declaredSuccess === undefined) {
      return declaredError
        ? err({
            kind: "http",
            status: response.status,
            body: value,
            headers: declaredError.headers
              ? responseHeaders(response, declaredError.headers, preserveWideIntegers)
              : response.headers,
          } as E)
        : err({ kind: "unexpected_http", status: response.status, body: value });
    }
    await captureCredential(client, response, provides, value, true);
    return ok(withSuccessStatus(value, response, declaredSuccess, preserveWideIntegers) as T);
  } catch (cause) {
    if (timedOut) return err({ kind: "timeout", afterMs: timeoutMs! });
    if (cause instanceof ResponseHeaderDecodeError || cause instanceof WirePolicyError) return err({ kind: "protocol", message: cause.message });
    if (cause instanceof CredentialSourceError) return err({ kind: "auth_source", scheme: cause.scheme, cause: cause.sourceCause });
    if (typeof DOMException !== "undefined" && cause instanceof DOMException && cause.name === "AbortError") {
      return err({ kind: "aborted", cause });
    }
    return err({ kind: "network", cause });
  } finally {
    if (timeoutId !== undefined) clearTimeout(timeoutId);
    if (timeoutController && parentSignal) parentSignal.removeEventListener("abort", abortParent);
  }
}

async function fetchWithRetry(
  fetchImpl: typeof fetch,
  url: URL,
  init: RequestInit,
  options: RetryOptions | undefined,
  idempotent: boolean,
  errors: Array<{ status: number | string; retryable?: boolean }>,
  signal: AbortSignal | undefined,
): Promise<Response> {
  const attempts = idempotent ? Math.max(1, Math.floor(options?.attempts ?? 1)) : 1;
  let attempt = 0;
  while (true) {
    let response: Response;
    try {
      response = await fetchImpl(url, init);
    } catch (cause) {
      if (attempt + 1 >= attempts) throw cause;
      await waitForRetry(retryDelay(undefined, attempt, options), signal);
      attempt += 1;
      continue;
    }
    const declaredError = response.status >= 400 ? findDeclared(errors, response.status) : undefined;
    const retryableStatus = declaredError?.retryable ?? (response.status === 502 || response.status === 503 || response.status === 504);
    if (!retryableStatus || attempt + 1 >= attempts) return response;
    try {
      await response.body?.cancel();
    } catch {
    }
    await waitForRetry(retryDelay(response, attempt, options), signal);
    attempt += 1;
  }
}

function retryDelay(response: Response | undefined, attempt: number, options: RetryOptions | undefined): number {
  const retryAfter = response?.headers.get("retry-after");
  const parsedRetryAfter = retryAfter === null || retryAfter === undefined ? undefined : Number(retryAfter);
  if (parsedRetryAfter !== undefined && Number.isFinite(parsedRetryAfter) && parsedRetryAfter >= 0) {
    return parsedRetryAfter * 1000;
  }
  const retryAfterDate = retryAfter === null || retryAfter === undefined ? undefined : Date.parse(retryAfter);
  if (retryAfterDate !== undefined && Number.isFinite(retryAfterDate)) {
    return Math.max(0, retryAfterDate - Date.now());
  }
  const backoff = Math.max(0, options?.backoffMs ?? 100);
  const maximum = Math.max(0, options?.maxBackoffMs ?? 1000);
  const delay = Math.min(maximum, backoff * 2 ** attempt);
  return delay + Math.max(0, options?.jitterMs ?? 0) * Math.random();
}

function waitForRetry(milliseconds: number, signal: AbortSignal | undefined): Promise<void> {
  if (signal?.aborted) return Promise.reject(abortError());
  if (milliseconds <= 0) return Promise.resolve();
  return new Promise((resolve, reject) => {
    let timer: ReturnType<typeof setTimeout>;
    let onAbort: () => void;
    const cleanup = () => signal?.removeEventListener("abort", onAbort);
    onAbort = () => {
      clearTimeout(timer);
      cleanup();
      reject(abortError());
    };
    timer = setTimeout(() => {
      cleanup();
      resolve();
    }, milliseconds);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function abortError(): Error {
  if (typeof DOMException !== "undefined") return new DOMException("The operation was aborted", "AbortError");
  return Object.assign(new Error("The operation was aborted"), { name: "AbortError" });
}

async function requestOrThrow<T, E>(operation: Promise<Result<T, E | ClientFailure>>): Promise<T> {
  const result = await operation;
  if (result.isErr()) throw result.error;
  return result.value as T;
}

function mediaTypeBase(value: string): string {
  return value.split(";", 1)[0].trim().toLowerCase();
}

function findDeclared<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`)
    ?? entries.find((entry) => entry.status === "default");
}

function findSuccess<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`)
    ?? (actual >= 200 && actual < 300 ? entries.find((entry) => entry.status === "unknown") : undefined)
    ?? (actual >= 200 && actual < 300 ? entries.find((entry) => entry.status === "default") : undefined);
}

function withSuccessStatus(
  value: unknown,
  response: Response,
  schema: { status: number | string; headers?: Array<{ name: string; required: boolean; schema: unknown }> } | undefined,
  preserveWideIntegers: boolean,
): unknown {
  const wrapped = withResponseHeaders(value, response, schema, preserveWideIntegers);
  return schema?.status === "unknown" || schema?.status === "default"
    ? { value: wrapped, status: response.status }
    : wrapped;
}

function withResponseHeaders(
  value: unknown,
  response: Response,
  schema: { headers?: Array<{ name: string; required: boolean; schema: unknown }> } | undefined,
  preserveWideIntegers: boolean,
): unknown {
  if (!schema?.headers) return value;
  return { value, headers: responseHeaders(response, schema.headers, preserveWideIntegers) };
}

function responseHeaders(
  response: Response,
  headers: Array<{ name: string; required: boolean; schema: unknown }>,
  preserveWideIntegers: boolean,
): Record<string, unknown> {
  const values: Record<string, unknown> = {};
  for (const header of headers) {
    const headerValue = response.headers.get(header.name);
    if (headerValue !== null) values[header.name] = decodeHeaderValue(headerValue, header.schema, preserveWideIntegers);
  }
  return values;
}

function decodeHeaderValue(value: string, schema: any, preserveWideIntegers: boolean): any {
  try {
    if (schema.ref) {
      const definition = typeDefinitions[schema.ref];
      if (definition?.kind === "newtype") return decodeHeaderValue(value, definition.inner, preserveWideIntegers);
      if (definition?.kind === "enum") return decodeWire(value, schema);
      return decodeWire(parseJson(value, preserveWideIntegers), schema);
    }
    if (schema.kind === "int") {
      if (schema.wire === "string") return value;
      return schema.bits > 53 && preserveWideIntegers ? BigInt(value) : parseNumber(value);
    }
    if (schema.kind === "float") return parseNumber(value);
    if (schema.kind === "decimal") return schema.wire === "number" ? parseNumber(value) : value;
    if (schema.kind === "bool") {
      if (value === "true") return true;
      if (value === "false") return false;
      throw new Error("invalid boolean");
    }
    if (schema.kind === "nullable") return decodeHeaderValue(value, schema.inner, preserveWideIntegers);
    if (schema.kind === "array" || schema.kind === "map" || schema.kind === "tuple") {
      return decodeWire(parseJson(value, preserveWideIntegers), schema);
    }
    return value;
  } catch (cause) {
    if (cause instanceof ResponseHeaderDecodeError) throw cause;
    throw new ResponseHeaderDecodeError(`invalid response header value ${JSON.stringify(value)}`);
  }
}

function parseNumber(value: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) throw new Error("invalid number");
  return parsed;
}

type SseStreamFailure = { kind: "stream_error"; cause: unknown } | { kind: "unknown_event"; event: string } | { kind: "invalid_event"; event: string; cause: unknown } | { kind: "server_error"; body: unknown };

async function* readSse<EventValue>(
  body: ReadableStream<Uint8Array>,
  events: Array<{ event: string; schema: unknown }>,
  preserveWideIntegers: boolean,
): AsyncIterable<Result<EventValue, SseStreamFailure>> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let eventName = "";
  let data: string[] = [];
  try {
    while (true) {
      const { done, value } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      if (done) buffer += "\n\n";
      let lineEnd = buffer.indexOf("\n");
      while (lineEnd >= 0) {
        let line = buffer.slice(0, lineEnd);
        buffer = buffer.slice(lineEnd + 1);
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line === "") {
          if (data.length) {
            const name = eventName || "message";
            const descriptor = events.find((entry) => entry.event === name);
            if (!descriptor) {
              yield err({ kind: "unknown_event", event: name });
            } else {
              try {
                const parsed = parseJson(data.join("\n"), preserveWideIntegers);
                const decoded = decodeWire(parsed, descriptor.schema);
                if (name === "error") yield err({ kind: "server_error", body: decoded });
                else yield ok({ event: name, data: decoded } as EventValue);
              } catch (cause) {
                yield err({ kind: "invalid_event", event: name, cause });
              }
            }
          }
          eventName = "";
          data = [];
        } else if (!line.startsWith(":")) {
          const separator = line.indexOf(":");
          const field = separator < 0 ? line : line.slice(0, separator);
          const value = separator < 0 ? "" : line.slice(separator + 1).replace(/^ /, "");
          if (field === "event") eventName = value;
          if (field === "data") data.push(value);
        }
        lineEnd = buffer.indexOf("\n");
      }
      if (done) break;
    }
  } catch (cause) {
    yield err({ kind: "stream_error", cause });
  } finally {
    try {
      await reader.cancel();
    } catch {}
    reader.releaseLock();
  }
}

type NdjsonStreamFailure = { kind: "stream_error"; cause: unknown } | { kind: "invalid_item"; cause: unknown };

async function* readNdjson<Item>(
  body: ReadableStream<Uint8Array>,
  schema: unknown,
  preserveWideIntegers: boolean,
): AsyncIterable<Result<Item, NdjsonStreamFailure>> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      if (done) buffer += "\n";
      let lineEnd = buffer.indexOf("\n");
      while (lineEnd >= 0) {
        let line = buffer.slice(0, lineEnd);
        buffer = buffer.slice(lineEnd + 1);
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line.trim() !== "") {
          try {
            const parsed = parseJson(line, preserveWideIntegers);
            yield ok(decodeWire(parsed, schema) as Item);
          } catch (cause) {
            yield err({ kind: "invalid_item", cause });
          }
        }
        lineEnd = buffer.indexOf("\n");
      }
      if (done) break;
    }
  } catch (cause) {
    yield err({ kind: "stream_error", cause });
  } finally {
    try {
      await reader.cancel();
    } catch {}
    reader.releaseLock();
  }
}

function parseJson(text: string, preserveWideIntegers: boolean): unknown {
  if (!preserveWideIntegers) return JSON.parse(text);
  const marker = "\u0000pons-bigint:";
  let output = "";
  let index = 0;
  let inString = false;
  while (index < text.length) {
    const character = text[index];
    if (inString) {
      output += character;
      if (character === "\\") {
        index += 1;
        if (index < text.length) output += text[index];
      } else if (character === "\"") {
        inString = false;
      }
      index += 1;
      continue;
    }
    if (character === "\"") {
      inString = true;
      output += character;
      index += 1;
      continue;
    }
    if (character === "-" || (character >= "0" && character <= "9")) {
      const start = index;
      if (character === "-") index += 1;
      while (index < text.length && text[index] >= "0" && text[index] <= "9") index += 1;
      if (text[index] === ".") {
        index += 1;
        while (index < text.length && text[index] >= "0" && text[index] <= "9") index += 1;
      }
      if (text[index] === "e" || text[index] === "E") {
        index += 1;
        if (text[index] === "+" || text[index] === "-") index += 1;
        while (index < text.length && text[index] >= "0" && text[index] <= "9") index += 1;
      }
      const token = text.slice(start, index);
      if (/^-?\d+$/.test(token) && !Number.isSafeInteger(Number(token))) {
        output += JSON.stringify(marker + token);
      } else {
        output += token;
      }
      continue;
    }
    output += character;
    index += 1;
  }
  return JSON.parse(output, (_key: string, value: unknown) => {
    if (typeof value === "string" && value.startsWith(marker)) {
      return BigInt(value.slice(marker.length));
    }
    return value;
  });
}

function stringifyJson(value: unknown): string {
  if (typeof value === "bigint") return value.toString();
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map((item) => stringifyJson(item)).join(",")}]`;
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return JSON.stringify(value) ?? "null";
  return `{${Object.entries(value as Record<string, unknown>)
    .filter(([, item]) => item !== undefined)
    .map(([key, item]) => `${JSON.stringify(key)}:${stringifyJson(item)}`)
    .join(",")}}`;
}

function encodeWire(value: any, schema: any): any {
  validateWire(value, schema);
  if (value == null || schema == null) return value;
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "encode");
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "encode");
    if (definition.kind === "enum") return transformEnum(value, definition, "encode");
    if (definition.kind === "newtype") return encodeWire(value, definition.inner);
    return value;
  }
  if (schema.kind === "bytes") return encodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return encodeWire(value, schema.inner);
  if (schema.kind === "array") return value.map((item: unknown) => encodeWire(item, schema.item));
  if (schema.kind === "tuple") return schema.items.map((item: unknown, index: number) => encodeWire(value[index], item));
  if (schema.kind === "map") return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, encodeWire(item, schema.value)]));
  return value;
}

function decodeWire(value: any, schema: any): any {
  validateWire(value, schema);
  if (value == null || schema == null) return value;
  if (schema.kind === "int" && schema.bits <= 53 && typeof value === "bigint") return Number(value);
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "decode");
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "decode");
    if (definition.kind === "enum") return transformEnum(value, definition, "decode");
    if (definition.kind === "newtype") return decodeWire(value, definition.inner);
    return value;
  }
  if (schema.kind === "bytes") return decodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return decodeWire(value, schema.inner);
  if (schema.kind === "array") return value.map((item: unknown) => decodeWire(item, schema.item));
  if (schema.kind === "tuple") return schema.items.map((item: unknown, index: number) => decodeWire(value[index], item));
  if (schema.kind === "map") return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, decodeWire(item, schema.value)]));
  return value;
}

function transformStruct(value: any, definition: any, mode: "encode" | "decode"): any {
  const fields = collectFields(definition);
  const flattenedMaps = collectMapValues(definition.flattened ?? []);
  const transform = mode === "encode" ? encodeWire : decodeWire;
  const known = new Set(fields.map((field: any) => field.wire));
  const openness = definition.openness ?? "preserve_opaque";
  const policy = typeof openness === "string" ? openness : Object.keys(openness)[0];
  if (policy === "reject") {
    const unknown = Object.keys(value).find((key) => !known.has(key));
    if (unknown !== undefined) throw new WirePolicyError(`unknown property ${unknown}`);
  }
  const output = policy === "reject" ? {} : { ...value };
  for (const field of fields) {
    if (Object.hasOwn(value, field.wire)) output[field.wire] = transform(value[field.wire], field.ty);
  }
  const additional = policy === "preserve_typed" ? openness.preserve_typed.value : flattenedMaps[0];
  if (additional) {
    for (const key of Object.keys(output)) {
      if (!known.has(key)) output[key] = transform(output[key], additional);
    }
  }
  return output;
}

function collectFields(definition: any): any[] {
  const fields = [...(definition.fields ?? [])];
  for (const reference of definition.flattened ?? []) {
    const flattened = reference.ref ? typeDefinitions[reference.ref] : undefined;
    if (flattened?.kind === "struct") fields.push(...collectFields(flattened));
    if (flattened?.kind === "newtype" && flattened.inner.ref) {
      const inner = typeDefinitions[flattened.inner.ref];
      if (inner?.kind === "struct") fields.push(...collectFields(inner));
    }
  }
  return fields;
}

function collectMapValues(references: any[]): any[] {
  return references.flatMap((reference) => {
    if (reference.kind === "map") return [reference.value];
    if (!reference.ref) return [];
    const definition = typeDefinitions[reference.ref];
    return definition?.kind === "newtype" && definition.inner.kind === "map"
      ? [definition.inner.value]
      : definition?.kind === "struct"
        ? collectMapValues(definition.flattened ?? [])
        : [];
  });
}

function transformEnum(value: any, definition: any, mode: "encode" | "decode"): any {
  const repr = definition.repr;
  let variant: any;
  let payload: unknown;
  if (repr.kind === "string") return value;
  if (repr.kind === "external") {
    if (typeof value === "string") return value;
    const wire = Object.keys(value)[0];
    variant = definition.variants.find((entry: any) => entry.wire === wire);
    payload = value[wire];
  } else if (repr.kind === "internal") {
    variant = definition.variants.find((entry: any) => entry.wire === value[repr.tag]);
    payload = value;
  } else if (repr.kind === "adjacent") {
    variant = definition.variants.find((entry: any) => entry.wire === value[repr.tag]);
    payload = value[repr.content];
  } else {
    return value;
  }
  return transformVariant(value, definition, variant, mode, payload);
}

function transformVariant(value: any, definition: any, variant: any, mode: "encode" | "decode", payloadValue: any = value): any {
  if (!variant || variant.payload.kind === "unit") return value;
  const payload = variant.payload;
  const transform = mode === "encode" ? encodeWire : decodeWire;
  const transformPayload = (current: any): any => {
    if (payload.kind === "newtype") return transform(current, payload.ty);
    if (payload.kind === "tuple") return payload.items.map((item: unknown, index: number) => transform(current[index], item));
    if (payload.kind === "struct") return transformStruct(current, { kind: "struct", fields: payload.fields }, mode);
    return current;
  };
  const repr = definition.repr;
  if (repr.kind === "external") {
    const wire = variant.wire;
    return typeof value === "string" ? value : { [wire]: transformPayload(payloadValue) };
  }
  if (repr.kind === "adjacent") return { ...value, [repr.content]: transformPayload(payloadValue) };
  return transformPayload(payloadValue);
}

function encodeBytes(value: Uint8Array | number[], encoding: string): string | number[] {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
  if (encoding === "array") return Array.from(bytes);
  if (encoding === "hex") return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join("");
  const base64 = btoa(binary);
  return encoding === "base64url" ? base64.replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "") : base64;
}

function decodeBytes(value: string | number[], encoding: string): Uint8Array {
  if (encoding === "array") return new Uint8Array(value as number[]);
  if (encoding === "hex") return new Uint8Array((value as string).match(/.{1,2}/g)?.map((byte) => parseInt(byte, 16)) ?? []);
  let base64 = value as string;
  if (encoding === "base64url") base64 = base64.replaceAll("-", "+").replaceAll("_", "/");
  base64 += "=".repeat((4 - base64.length % 4) % 4);
  return Uint8Array.from(atob(base64), (byte) => byte.charCodeAt(0));
}

function cookieValue(header: string | null, name: string): string | undefined {
  const cookie = header?.split(";").find((part) => part.trim().startsWith(`${name}=`))?.trim();
  return cookie?.slice(name.length + 1);
}

function pointerValue(value: unknown, pointer: string): unknown {
  if (pointer === "") return value;
  return pointer.split("/").slice(1).reduce<unknown>((current, part) => {
    const key = part.replaceAll("~1", "/").replaceAll("~0", "~");
    return current && typeof current === "object" ? (current as Record<string, unknown>)[key] : undefined;
  }, value);
}

"#
}

pub(super) fn render() -> String {
    format!(
        "{}{}{}",
        render_core(),
        crate::wire_validation::render(),
        crate::runtime_auth::render()
    )
}
