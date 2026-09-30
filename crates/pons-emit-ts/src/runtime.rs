fn render_core() -> &'static str {
    r#"class ResponseHeaderDecodeError extends Error {}
class WirePolicyError extends Error {}
class CredentialRedirectError extends Error {
  constructor(readonly schemes: string[]) {
    super(schemes.length > 0
      ? "redirect would forward credentials outside their scope"
      : "cross-origin redirect would carry request state outside the origin that produced it");
  }
}
class CredentialSourceError extends Error {
  constructor(readonly scheme: string, readonly sourceCause: unknown) {
    super(`credential source failed for ${scheme}`);
  }
}

class OpaqueCodecError extends Error {
  constructor(readonly codec: string, readonly phase: "encode" | "decode", readonly sourceCause: unknown) {
    super(`opaque codec failed for ${codec}`);
  }
}

type OpaqueContext = { codecs?: OpaqueCodecRegistry; required: boolean; allowUnsafeWideIntegers?: boolean };
type CredentialLocator =
  | { from: "body"; pointer: string }
  | { from: "header"; name: string }
  | { from: "set_cookie"; name: string };
type CredentialProvision = { scheme: string; value: CredentialLocator; expiresAt?: CredentialLocator };
type ConfiguredClient = ClientOptions & { authRecords: Record<string, CredentialValue[]>; authInvalidated: Record<string, boolean>; opaque: OpaqueContext };
const MAX_TIMER_DELAY = 2_147_483_647;
const MAX_REDIRECTS = 20;
const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
const MAX_STREAM_BUFFER = 8 * 1024 * 1024;
const MAX_STREAM_FIELDS = 100_000;

type RequestInputDescriptor = {
  path: Array<{ name: string; required: boolean; schema: unknown }>;
  query?: unknown;
  headers: Array<{ name: string; required: boolean; schema: unknown }>;
  bodyRequired: boolean;
};
type AttachedCustomCredential = { scheme: string; headers: string[]; query: Array<[string, string]>; cookies: string[] };
type AttachedQueryCredential = { scheme: string; params: Array<[string, string]> };

function dropCredentialScheme<T extends { scheme: string }>(credentials: T[], scheme: string): void {
  for (let index = credentials.length - 1; index >= 0; index -= 1) {
    if (credentials[index].scheme === scheme) credentials.splice(index, 1);
  }
}

function reattachQueryCredentials(
  url: URL,
  credentials: AttachedQueryCredential[],
  invalidated: Record<string, boolean>,
): void {
  for (const credential of credentials) {
    for (const [name, value] of credential.params) {
      if (invalidated[credential.scheme] === true) url.searchParams.delete(name);
      else url.searchParams.set(name, value);
    }
  }
}

function isCookieAuth(definition: { kind: string; location?: string } | undefined): boolean {
  return definition?.kind === "cookie" || (definition?.kind === "api_key" && definition.location === "cookie");
}

async function request<T, E>(
  client: ConfiguredClient,
  method: string,
  path: string | ((values: Record<string, unknown>) => string),
  pathValues: Record<string, unknown> | undefined,
  query: unknown,
  requestHeaders: Record<string, unknown> | undefined,
  body: unknown,
  content: string | undefined,
  mediaType: string | undefined,
  authGroups: string[][],
  authScopes: Record<string, readonly string[]>,
  bodySchema: unknown,
  inputs: RequestInputDescriptor,
  responses: Array<{ status: number | string; content: string; mediaType?: string; schema?: unknown; stream?: "sse" | "ndjson"; item?: unknown; events?: Array<{ event: string; schema: unknown }>; headers?: Array<{ name: string; required: boolean; schema: unknown }> }>,
  errors: Array<{ status: number | string; content: string; mediaType?: string; schema?: unknown; retryable?: boolean; events?: Array<{ event: string; schema: unknown }>; headers?: Array<{ name: string; required: boolean; schema: unknown }> }>,
  authOptional: boolean,
  preserveWideIntegers: boolean,
  provides: CredentialProvision | undefined,
  callOptions: CallOptions | undefined,
  idempotent: boolean,
): Promise<Result<T, E | ClientFailure>> {
  const configuredTimeout = callOptions?.timeout ?? client.timeout;
  const timeoutMs = typeof configuredTimeout === "number" && Number.isFinite(configuredTimeout)
    ? Math.min(Math.max(configuredTimeout, 0), MAX_TIMER_DELAY)
    : undefined;
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
    const encodedPathValues = preflightPath(pathValues, inputs.path, client.opaque);
    const encodedQuery = preflightQuery(query, inputs.query, client.opaque);
    const encodedRequestHeaders = preflightHeaders(requestHeaders, inputs.headers, client.opaque);
    if (inputs.bodyRequired && body === undefined) {
      throw new WirePolicyError("missing required request body");
    }
    const encodedBody = preflightBody(body, bodySchema, content, client.opaque);
    const baseUrl = client.baseUrl.endsWith("/") ? client.baseUrl : `${client.baseUrl}/`;
    const resolvedPath = typeof path === "function" ? path(encodedPathValues) : path;
    const url = new URL(resolvedPath.replace(/^\/+/, ""), baseUrl);
    if (encodedQuery && typeof encodedQuery === "object") {
      for (const [key, value] of Object.entries(encodedQuery)) {
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
    for (const [key, value] of Object.entries(encodedRequestHeaders)) {
      if (value !== undefined && value !== null) headers.set(key, stringifyCarrier(value));
    }
    let credentials: RequestCredentials | undefined;
    const credentialValues = new Map<string, CredentialValue>();
    const resolveCredential = async (scheme: string): Promise<CredentialValue> => {
      if (!credentialValues.has(scheme)) {
        credentialValues.set(scheme, await credentialValue(client, scheme, url));
      }
      return credentialValues.get(scheme)!;
    };
    let activeGroup: string[] | undefined;
    for (const group of authGroups) {
      if (group.length === 0) continue;
      let available = true;
      for (const scheme of group) {
        const definition = authSchemes[scheme];
        if (definition === undefined) {
          available = false;
          break;
        }
        if (definition.kind === "custom") {
          if (client.authAdapters?.[scheme] === undefined) {
            available = false;
            break;
          }
          continue;
        }
        const explicitCookieState = isCookieAuth(definition) && (
          client.auth?.[scheme] !== undefined ||
          credentialRecords(client, scheme).length > 0 ||
          client.authInvalidated[scheme] === true
        );
        const credential = await resolveCredential(scheme);
        const value = credential.value;
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
        if (value === undefined && (!isCookieAuth(definition) || explicitCookieState)) {
          available = false;
          break;
        }
      }
      if (available) {
        activeGroup = group;
        break;
      }
    }
    if (authGroups.length && !activeGroup && !authOptional) {
      return err({ kind: "auth", schemes: authGroups.flat() });
    }
     const attachedCredentialOrigins: Array<{ scheme: string; origin: string }> = [];
     const attachedCookies: Array<{ scheme: string; path?: string; secure?: boolean; host?: string }> = [];
     const attachedCustomCredentials: AttachedCustomCredential[] = [];
     const attachedQueryCredentials: AttachedQueryCredential[] = [];
    for (const scheme of activeGroup ?? []) {
      const definition = authSchemes[scheme];
      if (definition === undefined) continue;
      if (definition.kind === "custom") {
        const adapter = client.authAdapters?.[scheme];
        if (adapter === undefined) continue;
        try {
          const scopes = Object.prototype.hasOwnProperty.call(authScopes, scheme) ? [...(authScopes[scheme] ?? [])] : [];
          const attachment = await adapter({ scheme, scopes, method, url, headers });
           const customHeaders = Object.keys(attachment.headers ?? {});
           const customQuery = Object.entries(attachment.query ?? {});
           const customCookies = Object.keys(attachment.cookies ?? {});
           for (const [key, value] of Object.entries(attachment.headers ?? {})) headers.set(key, value);
           for (const [key, value] of Object.entries(attachment.query ?? {})) url.searchParams.set(key, value);
           for (const [key, value] of Object.entries(attachment.cookies ?? {})) setCookieHeader(headers, key, value, scheme);
           attachedCustomCredentials.push({ scheme, headers: customHeaders, query: customQuery, cookies: customCookies });
           if (customQuery.length > 0) attachedQueryCredentials.push({ scheme, params: customQuery });
           attachedCredentialOrigins.push({ scheme, origin: url.origin });
        } catch (cause) {
          return err({ kind: "auth_adapter", scheme, cause });
        }
        continue;
      }
      if (isCookieAuth(definition)) {
        credentials = "include";
        const credential = await resolveCredential(scheme);
        if (credential.value !== undefined) {
          setCookieHeader(headers, definition.name!, credential.value, scheme);
          attachedCookies.push({ scheme, path: credential.path, secure: credential.secure, host: credential.host });
        }
        continue;
      }
      const credential = await resolveCredential(scheme);
      const value = credential.value;
      if (value === undefined) {
        if (!authOptional) return err({ kind: "auth", schemes: [scheme] });
        continue;
      }
      attachedCredentialOrigins.push({ scheme, origin: url.origin });
      if (definition.kind === "bearer") headers.set("authorization", `Bearer ${value}`);
      else if (definition.kind === "basic") {
        const bytes = new TextEncoder().encode(value);
        const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join("");
        headers.set("authorization", `Basic ${btoa(binary)}`);
      }
      else if (definition.kind === "api_key" && definition.location === "header") headers.set(definition.name!, value);
      else if (definition.kind === "api_key" && definition.location === "query") {
        url.searchParams.set(definition.name!, value);
        attachedQueryCredentials.push({ scheme, params: [[definition.name!, value]] });
      }
      else if (definition.kind === "api_key" && definition.location === "cookie") setCookieHeader(headers, definition.name!, value, scheme);
    }
     const init: RequestInit = { method, headers, credentials, signal, redirect: "manual" };
    if (encodedBody !== undefined) {
      if (content === "json" || content === "problem_json") {
        headers.set("content-type", mediaType ?? (content === "problem_json" ? "application/problem+json" : "application/json"));
        init.body = stringifyJson(encodedBody);
      } else if (content === "form") {
        const form = new URLSearchParams();
        for (const [key, value] of Object.entries(encodedBody as Record<string, unknown>)) {
          if (value !== undefined && value !== null) form.set(key, stringifyCarrier(value));
        }
        headers.set("content-type", mediaType ?? "application/x-www-form-urlencoded");
        init.body = form;
      } else if (content === "multipart") {
        const form = new FormData();
        for (const [key, value] of Object.entries(encodedBody as Record<string, unknown>)) {
           if (value !== undefined && value !== null) {
             form.set(key, value instanceof Blob ? value : stringifyCarrier(value));
           }
        }
        init.body = form;
      } else if (content === "text") {
        headers.set("content-type", mediaType ?? "text/plain; charset=utf-8");
        init.body = stringifyCarrier(encodedBody);
      } else {
        if (mediaType) headers.set("content-type", mediaType);
        else if (content === "bytes") headers.set("content-type", "application/octet-stream");
        init.body = content === "bytes" && Array.isArray(encodedBody)
          ? new Uint8Array(encodedBody as number[])
          : encodedBody as BodyInit;
      }
    }
    if (client.interceptors?.request) {
      try {
        await client.interceptors.request({ url, init });
      } catch (cause) {
        return err({ kind: "interceptor", phase: "request", cause });
      }
    }
     init.redirect = "manual";
     for (const credential of attachedCredentialOrigins) {
      if (credential.origin !== url.origin) {
        return err({ kind: "auth", schemes: [credential.scheme] });
      }
    }
    for (const cookie of attachedCookies) {
      const definition = authSchemes[cookie.scheme];
      if (!isCookieAuth(definition)) continue;
      if (
        (cookie.host !== undefined && cookie.host !== url.hostname) ||
        (cookie.path !== undefined && !cookiePathMatches(cookie.path, url.pathname)) ||
        (cookie.secure && url.protocol !== "https:")
      ) {
        removeCookieHeader(headers, definition.name!);
        return err({ kind: "auth", schemes: [cookie.scheme] });
      }
    }
     const redirectResult = await fetchWithRedirect(
       client,
       provides,
       client.fetch ?? fetch,
       url,
       init,
       retry,
       idempotent,
       preserveWideIntegers,
       method,
       authScopes,
       responses,
       errors,
       signal,
       credentials,
       attachedCredentialOrigins,
       attachedCookies,
        attachedCustomCredentials,
        attachedQueryCredentials,
       );

     const response = redirectResult.response;
     const finalUrl = redirectResult.url;
     const finalInit = redirectResult.init;
     if (client.interceptors?.response) {
      try {
         await client.interceptors.response({ response, url: finalUrl, init: finalInit });
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
     const actualContentType = responseContent.trim().toLowerCase();
     const contentTypeMatches = responseKind === "bytes"
       ? mediaTypeMatches(actualContentType, expectedContentType ?? "")
       : responseMediaTypeMatches(actualContentType, expectedContentType ?? "");
     if (expectedContentType !== undefined && !contentTypeMatches) {
       return err({ kind: "protocol", message: `expected ${expectedContentType}, received ${responseContent || "no content type"}` });
     }
    if (declaredSuccess?.content === "stream") {
      if (!response.body) return err({ kind: "protocol", message: "the event stream has no response body" });
       await captureCredential(client, response, provides, undefined, false, finalUrl);
      if (declaredSuccess.stream === "ndjson") {
        return ok(withSuccessStatus(readNdjson(response.body, declaredSuccess.item, preserveWideIntegers, client.opaque), response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
      }
      return ok(withSuccessStatus(readSse(response.body, declaredSuccess.events ?? [], preserveWideIntegers, client.opaque), response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
    }
    const jsonResponse = responseKind === "json" || responseKind === "problem_json" ||
      (!responseKind && responseContent.includes("json"));
    const textResponse = responseKind === "text" ||
      (!responseKind && responseContent.startsWith("text/"));
     if (responseKind === "empty" && await responseHasBody(response)) {
       return err({ kind: "protocol", message: "empty response contains a body" });
     }
     let rawValue: unknown;
     if (response.status === 204 || response.status === 304 || responseKind === "empty") {
      rawValue = undefined;
    } else if (jsonResponse) {
      const text = await readResponseText(response, responseContent);
      try {
        rawValue = parseJson(text, preserveWideIntegers);
      } catch {
        return err({ kind: "protocol", message: "invalid JSON response" });
      }
    } else if (textResponse) {
      rawValue = await readResponseText(response, responseContent);
    } else {
      rawValue = await readBoundedBytes(response);
    }
    if (declaredSuccess !== undefined) {
       await captureCredential(client, response, provides, rawValue, true, finalUrl);
    }
    const value = responseSchema?.schema && (jsonResponse || textResponse || hasOpaque(responseSchema.schema))
      ? decodeWire(rawValue, responseSchema.schema, preserveWideIntegers, client.opaque)
      : rawValue;
    if (declaredSuccess === undefined) {
      return declaredError
        ? err({
            kind: "http",
            status: response.status,
            body: value,
            headers: declaredError.headers
              ? responseHeaders(response, declaredError.headers, preserveWideIntegers, client.opaque)
              : response.headers,
          } as E)
        : err({ kind: "unexpected_http", status: response.status, body: value });
    }
    return ok(withSuccessStatus(value, response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
  } catch (cause) {
    if (timedOut) return err({ kind: "timeout", afterMs: timeoutMs! });
     if (cause instanceof CredentialRedirectError) {
      return err(cause.schemes.length > 0
        ? { kind: "auth", schemes: cause.schemes }
        : { kind: "protocol", message: cause.message });
     }
     if (cause instanceof ResponseHeaderDecodeError || cause instanceof WirePolicyError) return err({ kind: "protocol", message: cause.message });
    if (cause instanceof CredentialSourceError) return err({ kind: "auth_source", scheme: cause.scheme, cause: cause.sourceCause });
    if (cause instanceof OpaqueCodecError) return err({ kind: "opaque_codec", codec: cause.codec, phase: cause.phase, cause: cause.sourceCause });
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
  const configuredAttempts = options?.attempts ?? 1;
  const attempts = idempotent && Number.isFinite(configuredAttempts) ? Math.max(1, Math.floor(configuredAttempts)) : 1;
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

async function fetchWithRedirect(
  client: ConfiguredClient,
  provides: CredentialProvision | undefined,
  fetchImpl: typeof fetch,
  url: URL,
  init: RequestInit,
  options: RetryOptions | undefined,
  idempotent: boolean,
  preserveWideIntegers: boolean,
  requestMethod: string,
  authScopes: Record<string, readonly string[]>,
  responses: Array<{ status: number | string }>,
  errors: Array<{ status: number | string; retryable?: boolean }>,
  signal: AbortSignal | undefined,
  credentials: RequestCredentials | undefined,
  attachedCredentialOrigins: Array<{ scheme: string; origin: string }>,
  attachedCookies: Array<{ scheme: string; path?: string; secure?: boolean; host?: string }>,
  attachedCustomCredentials: AttachedCustomCredential[],
  attachedQueryCredentials: AttachedQueryCredential[],
): Promise<{ response: Response; url: URL; init: RequestInit }> {
  let currentUrl = new URL(url);
  let currentInit: RequestInit = { ...init };
  const visited = new Set([currentUrl.href]);
  for (let redirectCount = 0; ; redirectCount += 1) {
    const response = await fetchWithRetry(fetchImpl, currentUrl, currentInit, options, idempotent, errors, signal);
    if (!REDIRECT_STATUSES.has(response.status) || declaresStatus(responses, response.status) || declaresStatus(errors, response.status)) {
      return { response, url: currentUrl, init: currentInit };
    }
    const location = response.headers.get("location");
    if (location === null) return { response, url: currentUrl, init: currentInit };
    try {
      let nextUrl: URL;
      try {
        nextUrl = new URL(location, currentUrl);
      } catch {
        throw new WirePolicyError("invalid redirect location");
      }
      if ((nextUrl.protocol !== "http:" && nextUrl.protocol !== "https:") || nextUrl.username !== "" || nextUrl.password !== "") {
        throw new WirePolicyError("redirect target is not an HTTP origin");
      }
      if (visited.has(nextUrl.href)) throw new WirePolicyError("redirect loop");
      if (redirectCount >= MAX_REDIRECTS) throw new WirePolicyError("too many redirects");
      const nextInit: RequestInit = { ...currentInit };
      const method = currentInit.method?.toUpperCase();
      if ((response.status === 303 && method !== "HEAD") || ((response.status === 301 || response.status === 302) && method === "POST")) {
        nextInit.method = "GET";
        delete nextInit.body;
        const nextHeaders = new Headers(nextInit.headers);
        nextHeaders.delete("content-type");
        nextHeaders.delete("content-length");
        nextInit.headers = nextHeaders;
      }
      const effectiveMethod = nextInit.method?.toUpperCase() ?? requestMethod;
      const sameOrigin = nextUrl.origin === currentUrl.origin;
      if (sameOrigin) {
        reattachQueryCredentials(nextUrl, attachedQueryCredentials, client.authInvalidated);
      }
      validateRedirectCredentials(nextUrl, currentUrl, nextInit, credentials, attachedCredentialOrigins, attachedCookies, attachedCustomCredentials);
      if (provides !== undefined && sameOrigin) {
      const needsRedirectBody = provides.value.from === "body" || provides.expiresAt?.from === "body";
      if (needsRedirectBody) {
        let redirectBody: unknown;
        let bodyAvailable = false;
        const clone = typeof response.clone === "function" ? response.clone() : undefined;
        if (clone !== undefined && clone.body !== null) {
          bodyAvailable = true;
          const contentType = (response.headers.get("content-type") ?? "").toLowerCase();
          if (contentType.includes("json")) {
            const text = await readResponseText(clone, contentType);
            if (text.trim().length > 0) redirectBody = parseJson(text, preserveWideIntegers);
          } else if (contentType.startsWith("text/")) {
            const text = await readResponseText(clone, contentType);
            if (text.length > 0) redirectBody = text;
          } else {
            redirectBody = await readBoundedBytes(clone);
          }
        }
        if (bodyAvailable) await captureCredential(client, response, provides, redirectBody, true, currentUrl);
      }
    }
      try {
        await response.body?.cancel();
      } catch {
      }
      const redirectedInit = await applyCapturedCredential(client, sameOrigin ? provides : undefined, nextUrl, nextInit, effectiveMethod, authScopes, attachedCookies, attachedCustomCredentials, attachedQueryCredentials);
      currentUrl = nextUrl;
      currentInit = redirectedInit;
      visited.add(nextUrl.href);
    } catch (cause) {
      try {
        await response.body?.cancel();
      } catch {
      }
      throw cause;
    }
  }
}

function validateRedirectCredentials(
  nextUrl: URL,
  currentUrl: URL,
  init: RequestInit,
  credentials: RequestCredentials | undefined,
  attachedCredentialOrigins: Array<{ scheme: string; origin: string }>,
  attachedCookies: Array<{ scheme: string; path?: string; secure?: boolean; host?: string }>,
  attachedCustomCredentials: AttachedCustomCredential[],
): void {
  const originChanged = nextUrl.origin !== currentUrl.origin;
  const wrongOriginSchemes = attachedCredentialOrigins
    .filter((credential) => credential.origin !== nextUrl.origin)
    .map((credential) => credential.scheme);
  if (originChanged && wrongOriginSchemes.length > 0) throw new CredentialRedirectError(wrongOriginSchemes);
  for (const cookie of attachedCookies) {
    if (
      (cookie.host !== undefined && cookie.host !== nextUrl.hostname) ||
      (cookie.path !== undefined && !cookiePathMatches(cookie.path, nextUrl.pathname)) ||
      (cookie.secure && nextUrl.protocol !== "https:")
    ) {
      throw new CredentialRedirectError([cookie.scheme]);
    }
  }
  if (!originChanged) return;
  if (init.body !== undefined && init.body !== null) throw new CredentialRedirectError([]);
  if (nextUrl.search !== "") throw new CredentialRedirectError([]);
  if (credentials === "include") throw new CredentialRedirectError([]);
  const headers = new Headers(init.headers);
  if (headers.has("authorization") || headers.has("cookie")) throw new CredentialRedirectError([]);
  for (const credential of attachedCustomCredentials) {
    if (credential.headers.some((name) => headers.has(name))) throw new CredentialRedirectError([credential.scheme]);
  }
}

async function applyCapturedCredential(
  client: ConfiguredClient,
  provides: CredentialProvision | undefined,
  url: URL,
  init: RequestInit,
  method: string,
  authScopes: Record<string, readonly string[]>,
  attachedCookies: Array<{ scheme: string; path?: string; secure?: boolean; host?: string }>,
  attachedCustomCredentials: AttachedCustomCredential[],
  attachedQueryCredentials: AttachedQueryCredential[],
): Promise<RequestInit> {
  if (provides === undefined) return init;
  const definition = authSchemes[provides.scheme];
  if (definition === undefined) return init;
  const nextInit: RequestInit = { ...init };
  const headers = new Headers(init.headers);
  const captured = latestCredentialRecord(client, provides.scheme);
  const value = captured?.value;
  const invalidated = client.authInvalidated[provides.scheme] === true;
  if (definition.kind === "custom") {
    const previous = attachedCustomCredentials.filter((credential) => credential.scheme === provides.scheme);
    for (const credential of previous) {
      for (const name of credential.headers) headers.delete(name);
      for (const [name] of credential.query) url.searchParams.delete(name);
      for (const name of credential.cookies) removeCookieHeader(headers, name);
    }
    dropCredentialScheme(attachedCustomCredentials, provides.scheme);
    dropCredentialScheme(attachedQueryCredentials, provides.scheme);
    if (!invalidated) {
      const adapter = client.authAdapters?.[provides.scheme];
      if (adapter !== undefined) {
        const scopes = Object.prototype.hasOwnProperty.call(authScopes, provides.scheme) ? [...(authScopes[provides.scheme] ?? [])] : [];
        const attachment = await adapter({ scheme: provides.scheme, scopes, method, url, headers });
        const nextCredential = {
          scheme: provides.scheme,
          headers: Object.keys(attachment.headers ?? {}),
          query: Object.entries(attachment.query ?? {}),
          cookies: Object.keys(attachment.cookies ?? {}),
        };
        for (const [key, headerValue] of Object.entries(attachment.headers ?? {})) headers.set(key, headerValue);
        for (const [key, queryValue] of Object.entries(attachment.query ?? {})) url.searchParams.set(key, queryValue);
        for (const [key, cookieValue] of Object.entries(attachment.cookies ?? {})) setCookieHeader(headers, key, cookieValue, provides.scheme);
        attachedCustomCredentials.push(nextCredential);
        if (nextCredential.query.length > 0) {
          attachedQueryCredentials.push({ scheme: provides.scheme, params: nextCredential.query });
        }
      }
    }
  } else if (definition.kind === "bearer" || definition.kind === "basic") {
    if (invalidated) headers.delete("authorization");
    else if (value !== undefined) {
      if (definition.kind === "bearer") headers.set("authorization", `Bearer ${value}`);
      else {
        const bytes = new TextEncoder().encode(value);
        const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join("");
        headers.set("authorization", `Basic ${btoa(binary)}`);
      }
    }
  } else if (definition.kind === "api_key" && definition.location === "header") {
    if (invalidated) headers.delete(definition.name!);
    else if (value !== undefined) headers.set(definition.name!, value);
  } else if (definition.kind === "api_key" && definition.location === "query") {
    if (invalidated) url.searchParams.delete(definition.name!);
    else if (value !== undefined) url.searchParams.set(definition.name!, value);
  } else if (isCookieAuth(definition)) {
    const name = definition.name!;
    const cookieIndex = attachedCookies.findIndex((cookie) => cookie.scheme === provides.scheme);
    if (invalidated) {
      removeCookieHeader(headers, name);
      if (cookieIndex >= 0) attachedCookies.splice(cookieIndex, 1);
    } else if (value !== undefined) {
      const origin = captured?.origin;
      const host = captured?.host;
      const path = captured?.path;
      const secure = captured?.secure === true;
      if ((origin !== undefined && origin !== url.origin) || (host !== undefined && host !== url.hostname) || (path !== undefined && !cookiePathMatches(path, url.pathname)) || (secure && url.protocol !== "https:")) {
        throw new CredentialRedirectError([provides.scheme]);
      }
      setCookieHeader(headers, name, value, provides.scheme);
      const scope = { scheme: provides.scheme, path, secure, host };
      if (cookieIndex >= 0) Object.assign(attachedCookies[cookieIndex], scope);
      else attachedCookies.push(scope);
    }
  }
  nextInit.headers = headers;
  return nextInit;
}

function retryDelay(response: Response | undefined, attempt: number, options: RetryOptions | undefined): number {
  const retryAfter = response?.headers.get("retry-after");
  const retryAfterText = retryAfter?.trim();
  const parsedRetryAfter = retryAfterText !== undefined && /^\d+$/.test(retryAfterText) ? Number(retryAfterText) : undefined;
  if (parsedRetryAfter !== undefined && Number.isFinite(parsedRetryAfter)) {
    return parsedRetryAfter * 1000;
  }
  const retryAfterDate = retryAfterText !== undefined && /[A-Za-z]/.test(retryAfterText) ? Date.parse(retryAfterText) : undefined;
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
  const delay = Number.isFinite(milliseconds) ? Math.max(0, milliseconds) : MAX_TIMER_DELAY;
  if (delay === 0) return Promise.resolve();
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
      if (delay > MAX_TIMER_DELAY) resolve(waitForRetry(delay - MAX_TIMER_DELAY, signal));
      else resolve();
    }, Math.min(delay, MAX_TIMER_DELAY));
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

function encodePathSegment(value: string): string {
  if (value === "." || value === "..") throw new WirePolicyError("path segment cannot be a dot segment");
  return encodeURIComponent(value);
}

function mediaTypeBase(value: string): string {
  return value.split(";", 1)[0].trim().toLowerCase();
}

function responseMediaTypeMatches(actual: string, expected: string): boolean {
  return mediaTypeBase(actual) === mediaTypeBase(expected);
}

function normalizeMediaParameter(part: string): [string, string] {
  const [key, ...value] = part.split("=");
  return [key.trim().toLowerCase(), value.join("=").trim().toLowerCase()];
}

function mediaTypeMatches(actual: string, expected: string): boolean {
  if (mediaTypeBase(actual) !== mediaTypeBase(expected)) return false;
  const actualParameters = new Map(actual.split(";").slice(1).map(normalizeMediaParameter));
  return expected.split(";").slice(1).every((part) => {
    const [key, value] = normalizeMediaParameter(part);
    return actualParameters.get(key) === value;
  });
}

function copyRecord(value: Record<string, unknown>): Record<string, unknown> {
  const output: Record<string, unknown> = Object.create(null);
  for (const [key, item] of Object.entries(value)) output[key] = item;
  return output;
}

function preflightPath(
  values: Record<string, unknown> | undefined,
  descriptors: Array<{ name: string; required: boolean; schema: unknown }>,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const output = copyRecord(values ?? {});
  for (const descriptor of descriptors) {
    const present = Object.hasOwn(output, descriptor.name);
    const value = output[descriptor.name];
    if (!present || value === undefined) {
      if (descriptor.required) {
        throw new WirePolicyError(`missing required path parameter ${descriptor.name}`);
      }
      continue;
    }
    output[descriptor.name] = encodeWire(value, descriptor.schema, opaque);
  }
  return output;
}

function preflightQuery(value: unknown, schema: unknown, opaque: OpaqueContext): unknown {
  if (schema == null) return value;
  if (value === undefined) throw new WirePolicyError("missing request query");
  return encodeWire(value, schema, opaque);
}

function preflightHeaders(
  values: Record<string, unknown> | undefined,
  descriptors: Array<{ name: string; required: boolean; schema: unknown }>,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const output = copyRecord(values ?? {});
  for (const descriptor of descriptors) {
    const present = Object.hasOwn(output, descriptor.name);
    const value = output[descriptor.name];
    if (!present || value === undefined || value === null) {
      if (descriptor.required) {
        throw new WirePolicyError(`missing required request header ${descriptor.name}`);
      }
      delete output[descriptor.name];
      continue;
    }
    output[descriptor.name] = encodeWire(value, descriptor.schema, opaque);
  }
  return output;
}

function preflightBody(value: unknown, schema: unknown, content: string | undefined, opaque: OpaqueContext): unknown {
  if (value === undefined || schema == null) return value;
  if (["json", "problem_json", "form", "multipart"].includes(content ?? "") || hasOpaque(schema)) {
    return encodeWire(value, schema, opaque);
  }
  validateWire(value, schema, "encode", opaque.allowUnsafeWideIntegers === true);
  return value;
}

function declaresStatus<T extends { status: number | string }>(entries: T[], actual: number): boolean {
  return entries.some(
    (entry) => entry.status === actual
      || (typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`),
  );
}

function findDeclared<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`)
    ?? entries.find((entry) => entry.status === "default");
}

function findSuccess<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`)
    ?? entries.find((entry) => entry.status === "unknown")
    ?? entries.find((entry) => entry.status === "default");
}

function withSuccessStatus(
  value: unknown,
  response: Response,
  schema: { status: number | string; headers?: Array<{ name: string; required: boolean; schema: unknown }> } | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): unknown {
  const wrapped = withResponseHeaders(value, response, schema, preserveWideIntegers, opaque);
  return schema?.status === "unknown" || schema?.status === "default"
    ? { value: wrapped, status: response.status }
    : wrapped;
}

function withResponseHeaders(
  value: unknown,
  response: Response,
  schema: { headers?: Array<{ name: string; required: boolean; schema: unknown }> } | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): unknown {
  if (!schema?.headers) return value;
  return { value, headers: responseHeaders(response, schema.headers, preserveWideIntegers, opaque) };
}

function responseHeaders(
  response: Response,
  headers: Array<{ name: string; required: boolean; schema: unknown }>,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const values: Record<string, unknown> = Object.create(null);
  for (const header of headers) {
    const headerValue = response.headers.get(header.name);
    if (headerValue !== null) values[header.name] = decodeHeaderValue(headerValue, header.schema, preserveWideIntegers, opaque);
  }
  return values;
}

function decodeHeaderValue(value: string, schema: any, preserveWideIntegers: boolean, opaque: OpaqueContext): any {
  try {
    if (schema.ref) {
       const definition = typeDefinitions[schema.ref];
       if (definition && !definitionApplies(definition, "decode")) {
         throw new WirePolicyError("response header type cannot be used in this direction");
       }
        if (definition?.kind === "newtype") return decodeHeaderValue(value, definition.inner, preserveWideIntegers, opaque);
        if (definition?.kind === "bytes") return decodeHeaderValue(value, definition, preserveWideIntegers, opaque);
        if (definition?.kind === "enum") {
         const parsed = definition.repr.kind === "string" || (definition.repr.kind === "external" && !value.trim().startsWith("{"))
           ? value
           : parseJson(value, preserveWideIntegers);
         return decodeWire(parsed, schema, preserveWideIntegers, opaque);
       }
      return decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque);
    }
     if (schema.kind === "opaque") return decodeWire(value, schema, preserveWideIntegers, opaque);
     if (schema.kind === "bytes") {
       const parsed = schema.encoding === "array" ? parseJson(value, false) : value;
       return decodeWire(parsed, schema, preserveWideIntegers, opaque);
     }
     if (schema.kind === "int") {
       const parsed = schema.wire === "string"
         ? parseIntegerText(value)
         : parseIntegerNumber(value, schema.bits, preserveWideIntegers);
       return decodeWire(parsed, schema, preserveWideIntegers, opaque);
     }
     if (schema.kind === "float") return decodeWire(parseNumber(value), schema, preserveWideIntegers, opaque);
     if (schema.kind === "decimal") {
       return schema.wire === "number"
         ? decodeWire(parseNumber(value), schema, preserveWideIntegers, opaque)
         : decodeWire(value, schema, preserveWideIntegers, opaque);
     }
    if (schema.kind === "bool") {
      if (value === "true") return true;
      if (value === "false") return false;
      throw new Error("invalid boolean");
    }
    if (schema.kind === "nullable") return decodeHeaderValue(value, schema.inner, preserveWideIntegers, opaque);
     if (schema.kind === "array" || schema.kind === "map" || schema.kind === "tuple") {
       return decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque);
     }
     if (schema.kind === "string") return decodeWire(value, schema, preserveWideIntegers, opaque);
     return value;
  } catch (cause) {
    if (cause instanceof ResponseHeaderDecodeError || cause instanceof OpaqueCodecError) throw cause;
    throw new ResponseHeaderDecodeError(`invalid response header value ${JSON.stringify(value)}`);
  }
}

async function responseHasBody(response: Response): Promise<boolean> {
  if (!response.body) return false;
  const reader = response.body.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) return false;
      if (value.byteLength > 0) return true;
    }
  } finally {
    try {
      await reader.cancel();
    } catch {
    }
    reader.releaseLock();
  }
}

async function readBoundedBytes(response: Response): Promise<Uint8Array> {
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > MAX_STREAM_BUFFER) throw new WirePolicyError("response body exceeds the size limit");
    return bytes;
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > MAX_STREAM_BUFFER) {
        void reader.cancel().catch(() => {});
        throw new WirePolicyError("response body exceeds the size limit");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const output = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return output;
}

async function readResponseText(response: Response, contentType: string): Promise<string> {
  const match = /;\s*charset\s*=\s*(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
  const charset = (match?.[1] ?? match?.[2] ?? "utf-8").toLowerCase();
  if (!response.body && (charset === "utf-8" || charset === "utf8" || charset === "us-ascii")) return response.text();
  try {
    return new TextDecoder(charset).decode(await readBoundedBytes(response));
  } catch (cause) {
    if (cause instanceof WirePolicyError) throw cause;
    throw new WirePolicyError(`unsupported response charset ${charset}`);
  }
}

function parseIntegerText(value: string): string {
  const normalized = value.trim();
  if (!/^-?\d+$/.test(normalized)) throw new Error("invalid integer");
  return normalized;
}

function parseIntegerNumber(value: string, bits: number, preserveWideIntegers: boolean): number | bigint {
  const integer = BigInt(parseIntegerText(value));
  if (bits > 53 && preserveWideIntegers) return integer;
  return Number(integer);
}

function parseNumber(value: string): number {
  const normalized = value.trim();
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(normalized)) {
    throw new Error("invalid number");
  }
  const parsed = Number(normalized);
  if (!Number.isFinite(parsed)) throw new Error("invalid number");
  return parsed;
}

type SseStreamFailure = { kind: "stream_error"; cause: unknown } | { kind: "unknown_event"; event: string } | { kind: "invalid_event"; event: string; cause: unknown } | { kind: "server_error"; body: unknown };

function nextLineEnd(buffer: string, done: boolean): number {
  const lineFeed = buffer.indexOf("\n");
  const carriageReturn = buffer.indexOf("\r");
  const lineEnd = lineFeed < 0
    ? carriageReturn
    : carriageReturn < 0
      ? lineFeed
      : Math.min(lineFeed, carriageReturn);
  if (!done && lineEnd === buffer.length - 1 && buffer[lineEnd] === "\r") return -1;
  return lineEnd;
}

async function* readSse<EventValue>(
  body: ReadableStream<Uint8Array>,
  events: Array<{ event: string; schema: unknown }>,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): AsyncIterable<Result<EventValue, SseStreamFailure>> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
   let eventName = "";
   let data: string[] = [];
   let dataLength = 0;
   let fieldCount = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
       buffer += decoder.decode(value, { stream: !done });
       if (buffer.length > MAX_STREAM_BUFFER) throw new WirePolicyError("stream line exceeds the size limit");
       if (done) buffer += "\n\n";
       let lineEnd = nextLineEnd(buffer, done);
       while (lineEnd >= 0) {
         let line = buffer.slice(0, lineEnd);
         const delimiterLength = buffer[lineEnd] === "\r" && buffer[lineEnd + 1] === "\n" ? 2 : 1;
         buffer = buffer.slice(lineEnd + delimiterLength);
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
                const decoded = decodeWire(parsed, descriptor.schema, preserveWideIntegers, opaque);
                if (name === "error") yield err({ kind: "server_error", body: decoded });
                else yield ok({ event: name, data: decoded } as EventValue);
              } catch (cause) {
                yield err({ kind: "invalid_event", event: name, cause });
              }
            }
          }
            eventName = "";
            data = [];
            dataLength = 0;
            fieldCount = 0;
          } else if (!line.startsWith(":")) {
          fieldCount += 1;
          if (fieldCount > MAX_STREAM_FIELDS) throw new WirePolicyError("stream has too many fields");
          const separator = line.indexOf(":");
          const field = separator < 0 ? line : line.slice(0, separator);
          const value = separator < 0 ? "" : line.slice(separator + 1).replace(/^ /, "");
           if (field === "event") eventName = value;
           if (field === "data") {
             dataLength += value.length + 1;
             if (dataLength > MAX_STREAM_BUFFER) throw new WirePolicyError("stream event exceeds the size limit");
             data.push(value);
           }
        }
        lineEnd = nextLineEnd(buffer, done);
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
  opaque: OpaqueContext,
): AsyncIterable<Result<Item, NdjsonStreamFailure>> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
       buffer += decoder.decode(value, { stream: !done });
       if (buffer.length > MAX_STREAM_BUFFER) throw new WirePolicyError("stream item exceeds the size limit");
       if (done) buffer += "\n";
      let lineEnd = nextLineEnd(buffer, done);
      while (lineEnd >= 0) {
        let line = buffer.slice(0, lineEnd);
        const delimiterLength = buffer[lineEnd] === "\r" && buffer[lineEnd + 1] === "\n" ? 2 : 1;
        buffer = buffer.slice(lineEnd + delimiterLength);
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line.trim() !== "") {
          try {
            const parsed = parseJson(line, preserveWideIntegers);
            yield ok(decodeWire(parsed, schema, preserveWideIntegers, opaque) as Item);
          } catch (cause) {
            yield err({ kind: "invalid_item", cause });
          }
        }
        lineEnd = nextLineEnd(buffer, done);
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

function exactIntegerToken(token: string): bigint | undefined {
  const match = /^(-?)(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$/.exec(token);
  if (match === null) return undefined;
  const fraction = match[3] ?? "";
  const exponent = Number(match[4] ?? "0");
  if (!Number.isInteger(exponent) || Math.abs(exponent) > 10_000) return undefined;
  const coefficient = BigInt(match[2] + fraction);
  const scale = fraction.length - exponent;
  if (scale <= 0) {
    const magnitude = coefficient * (10n ** BigInt(-scale));
    return match[1] === "-" ? -magnitude : magnitude;
  }
  const divisor = 10n ** BigInt(scale);
  if (coefficient % divisor !== 0n) return undefined;
  const magnitude = coefficient / divisor;
  return match[1] === "-" ? -magnitude : magnitude;
}

function parseJson(text: string, preserveWideIntegers: boolean): unknown {
  if (!preserveWideIntegers) return JSON.parse(text);
  const strings: string[] = [];
  const pending: unknown[] = [JSON.parse(text)];
  while (pending.length > 0) {
    const value = pending.pop();
    if (typeof value === "string") {
      strings.push(value);
    } else if (Array.isArray(value)) {
      for (const item of value) pending.push(item);
    } else if (value !== null && typeof value === "object") {
      for (const [key, item] of Object.entries(value)) {
        strings.push(key);
        pending.push(item);
      }
    }
  }
  let marker = "\u0000pons-bigint:";
  while (strings.some((value) => value.startsWith(marker))) marker += "\u0000";
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
      const exactInteger = exactIntegerToken(token);
      if (exactInteger !== undefined && !Number.isSafeInteger(Number(token))) {
        output += JSON.stringify(marker + exactInteger.toString());
      } else {
        output += token;
      }
      continue;
    }
    output += character;
    index += 1;
  }
  return JSON.parse(output, (_key: string, value: unknown) => {
    if (typeof value !== "string" || !value.startsWith(marker)) return value;
    const token = value.slice(marker.length);
    return /^-?\d+$/.test(token) ? BigInt(token) : value;
  });
}

function stringifyCarrier(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "bigint") return value.toString();
  return stringifyJson(value);
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

function hasOpaque(schema: any, seen = new Set<string>()): boolean {
  if (schema == null) return false;
  if (schema.kind === "opaque") return true;
  if (schema.ref) {
    if (seen.has(schema.ref)) return false;
    const next = new Set(seen);
    next.add(schema.ref);
    return hasOpaque(typeDefinitions[schema.ref], next);
  }
  if (schema.kind === "nullable") return hasOpaque(schema.inner, seen);
  if (schema.kind === "array") return hasOpaque(schema.item, seen);
  if (schema.kind === "map") return hasOpaque(schema.value, seen);
  if (schema.kind === "tuple") return schema.items.some((item: any) => hasOpaque(item, seen));
  if (schema.kind === "struct") {
    return (schema.fields ?? []).some((field: any) => hasOpaque(field.ty, seen))
      || (schema.flattened ?? []).some((item: any) => hasOpaque(item, seen))
      || hasOpaque(schema.openness?.preserve_typed?.value, seen);
  }
  if (schema.kind === "enum") {
    return (schema.variants ?? []).some((variant: any) => hasOpaque(variant.payload, seen));
  }
  if (schema.kind === "newtype") return hasOpaque(schema.inner, seen);
  return false;
}

function flattenedOpaqueSchema(schema: any, seen = new Set<string>()): any | undefined {
  if (schema?.kind === "opaque") return schema;
  if (!schema?.ref || seen.has(schema.ref)) return undefined;
  const definition = typeDefinitions[schema.ref];
  if (definition?.kind !== "newtype" && definition?.kind !== "struct") return undefined;
  const next = new Set(seen);
  next.add(schema.ref);
  if (definition.kind === "newtype") return flattenedOpaqueSchema(definition.inner, next);
  for (const item of definition.flattened ?? []) {
    const nested = flattenedOpaqueSchema(item, next);
    if (nested !== undefined) return nested;
  }
  return undefined;
}

function isFlattenedOpaque(schema: any): boolean {
  return flattenedOpaqueSchema(schema) !== undefined;
}

function transformOpaque(
  value: any,
  schema: any,
  mode: "encode" | "decode",
  opaque: OpaqueContext,
): any {
  const hasCodec = Object.hasOwn(opaque.codecs ?? {}, schema.codec);
  const codec = hasCodec ? opaque.codecs?.[schema.codec] : undefined;
  if (!hasCodec || codec === undefined) {
    if (opaque.required) throw new OpaqueCodecError(schema.codec, mode, new Error("missing opaque codec"));
    return value;
  }
  try {
    return mode === "encode" ? codec.encode(value, schema.wire) : codec.decode(value, schema.wire);
  } catch (cause) {
    if (cause instanceof OpaqueCodecError) throw cause;
    throw new OpaqueCodecError(schema.codec, mode, cause);
  }
}

function encodeWire(value: any, schema: any, opaque: OpaqueContext): any {
  const selectedVariant = validateWire(value, schema, "encode", opaque.allowUnsafeWideIntegers === true);
  if (value == null || schema == null) return value;
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "encode", false, opaque);
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "encode", false, opaque);
    if (definition.kind === "enum") return transformEnum(value, definition, "encode", selectedVariant, false, opaque);
    if (definition.kind === "newtype") return encodeWire(value, definition.inner, opaque);
    return value;
  }
  if (schema.kind === "opaque") return transformOpaque(value, schema, "encode", opaque);
  if (schema.kind === "bytes") return encodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return encodeWire(value, schema.inner, opaque);
  if (schema.kind === "array") return value.map((item: unknown) => encodeWire(item, schema.item, opaque));
  if (schema.kind === "tuple") return schema.items.map((item: unknown, index: number) => encodeWire(value[index], item, opaque));
   if (schema.kind === "map") {
     const output: Record<string, unknown> = Object.create(null);
     for (const [key, item] of Object.entries(value)) output[key] = encodeWire(item, schema.value, opaque);
     return output;
   }
  return value;
}

function decodeWire(value: any, schema: any, preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
  const selectedVariant = validateWire(value, schema, "decode", opaque.allowUnsafeWideIntegers === true);
  if (schema?.kind === "float" && typeof value === "bigint") return Number(value);
  if (schema?.kind === "decimal" && schema.wire === "number" && typeof value === "bigint") return Number(value);
  if (value == null || schema == null) return value;
  if (schema.kind === "int") {
    if (schema.bits <= 53 && typeof value === "bigint") return Number(value);
    if (schema.wire === "number" && schema.bits > 53 && preserveWideIntegers && typeof value === "number") return BigInt(value);
  }
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "decode", preserveWideIntegers, opaque);
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "decode", preserveWideIntegers, opaque);
    if (definition.kind === "enum") return transformEnum(value, definition, "decode", selectedVariant, preserveWideIntegers, opaque);
    if (definition.kind === "newtype") return decodeWire(value, definition.inner, preserveWideIntegers, opaque);
    return value;
  }
  if (schema.kind === "opaque") return transformOpaque(value, schema, "decode", opaque);
  if (schema.kind === "bytes") return decodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return decodeWire(value, schema.inner, preserveWideIntegers, opaque);
  if (schema.kind === "array") return value.map((item: unknown) => decodeWire(item, schema.item, preserveWideIntegers, opaque));
  if (schema.kind === "tuple") return value.map((item: unknown, index: number) => decodeWire(value[index], item, preserveWideIntegers, opaque));
   if (schema.kind === "map") {
     const output: Record<string, unknown> = Object.create(null);
     for (const [key, item] of Object.entries(value)) output[key] = decodeWire(item, schema.value, preserveWideIntegers, opaque);
     return output;
   }
  return value;
}

function transformStruct(value: any, definition: any, mode: "encode" | "decode", preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
  const transform = (item: unknown, itemSchema: any) => mode === "encode"
    ? encodeWire(item, itemSchema, opaque)
    : decodeWire(item, itemSchema, preserveWideIntegers, opaque);
  let current = value;
  for (const item of definition.flattened ?? []) {
    const carrier = flattenedOpaqueSchema(item);
    if (carrier !== undefined) {
      current = transform(current, carrier);
      break;
    }
  }
  if (!isRecord(current)) return current;
  const fields = collectFields(definition, mode);
  const declaredFields = collectFields(definition);
  const flattenedOpaque = (definition.flattened ?? []).some((item: any) => isFlattenedOpaque(item));
  const flattenedMaps = collectMapValues(definition.flattened ?? []);
  const known = new Set(declaredFields.map((field: any) => field.wire));
  const openness = definition.openness ?? "preserve_opaque";
  const policy = typeof openness === "string" ? openness : Object.keys(openness)[0];
  if (policy === "reject") {
    const unknown = Object.keys(current).find((key) => !known.has(key));
    if (unknown !== undefined) throw new WirePolicyError(`unknown property ${unknown}`);
  }
  const output: Record<string, unknown> = Object.create(null);
  for (const key of Object.keys(current)) {
    if (!known.has(key)) output[key] = current[key];
  }
  for (const field of fields) {
    if (Object.hasOwn(current, field.wire) && current[field.wire] !== undefined) {
      output[field.wire] = !flattenedOpaque || hasOpaque(field.ty)
        ? transform(current[field.wire], field.ty)
        : current[field.wire];
    }
  }
  const additional = policy === "preserve_typed" ? opennessValue(openness.preserve_typed.value) : flattenedMaps[0];
  if (additional) {
    for (const key of Object.keys(output)) {
      if (!known.has(key) && output[key] !== undefined) output[key] = transform(output[key], additional);
    }
  }
  return output;
}

function collectFields(
  definition: any,
  mode?: "encode" | "decode",
  seen = new Set<any>(),
): any[] {
  if (!definitionApplies(definition, mode) || seen.has(definition)) return [];
  const next = new Set(seen);
  next.add(definition);
  const fields = (definition.fields ?? []).filter((field: any) => fieldApplies(field, definition, mode));
  for (const reference of definition.flattened ?? []) {
     const flattened = reference.ref ? typeDefinitions[reference.ref] : undefined;
     if (flattened && !definitionApplies(flattened, mode)) {
       throw new WirePolicyError("flattened type cannot be used in this direction");
     }
     if (flattened?.kind === "struct") fields.push(...collectFields(flattened, mode, next));
     if (flattened?.kind === "newtype") {
      let nested = flattened;
      while (nested?.kind === "newtype" && nested.inner.ref && !next.has(nested)) {
         next.add(nested);
         nested = typeDefinitions[nested.inner.ref];
         if (nested && !definitionApplies(nested, mode)) {
           throw new WirePolicyError("flattened type cannot be used in this direction");
         }
       }
       if (nested?.kind === "struct") fields.push(...collectFields(nested, mode, next));
    }
  }
  return fields;
}

function definitionApplies(definition: any, mode?: "encode" | "decode"): boolean {
  if (mode === undefined) return true;
  const direction = definition.direction ?? "both";
  return mode === "encode" ? direction !== "output" : direction !== "input";
}

function fieldApplies(field: any, definition: any, mode?: "encode" | "decode"): boolean {
  if (!definitionApplies(definition, mode)) return false;
  if (mode === "encode" && field.access === "read_only") return false;
  return mode !== "decode" || field.access !== "write_only";
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

function opennessValue(schema: any, seen = new Set<string>()): any {
  if (schema?.kind === "map") return schema.value;
  if (!schema?.ref || seen.has(schema.ref)) return schema;
  const definition = typeDefinitions[schema.ref];
  if (definition?.kind !== "newtype") return schema;
  seen.add(schema.ref);
  return opennessValue(definition.inner, seen);
}

function transformEnum(value: any, definition: any, mode: "encode" | "decode", selectedVariant?: any, preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
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
    const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
    variant = definition.variants.find((entry: any) => entry.wire === tag);
    payload = value;
  } else if (repr.kind === "adjacent") {
    const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
    variant = definition.variants.find((entry: any) => entry.wire === tag);
    payload = Object.hasOwn(value, repr.content) ? value[repr.content] : undefined;
  } else {
     variant = selectedVariant ?? selectUntaggedVariant(value, definition, undefined, mode, opaque.allowUnsafeWideIntegers === true);
    payload = value;
  }
  return transformVariant(value, definition, variant, mode, preserveWideIntegers, opaque, payload);
}

function transformVariant(value: any, definition: any, variant: any, mode: "encode" | "decode", preserveWideIntegers = false, opaque: OpaqueContext = { required: false }, payloadValue: any = value): any {
  if (!variant) return value;
  if (variant.payload.kind === "unit") return isRecord(value) ? copyRecord(value) : value;
  const payload = variant.payload;
  const transform = (item: unknown, itemSchema: any) => mode === "encode"
    ? encodeWire(item, itemSchema, opaque)
    : decodeWire(item, itemSchema, preserveWideIntegers, opaque);
  const transformPayload = (current: any): any => {
    if (payload.kind === "newtype") return transform(current, payload.ty);
    if (payload.kind === "tuple") return payload.items.map((item: unknown, index: number) => transform(current[index], item));
    if (payload.kind === "struct") return transformStruct(current, { kind: "struct", direction: definition.direction, fields: payload.fields }, mode, preserveWideIntegers, opaque);
    return current;
  };
  const repr = definition.repr;
  if (repr.kind === "internal" && isRecord(payloadValue)) {
    const { [repr.tag]: _tag, ...payloadObject } = payloadValue as any;
    const transformed = transformPayload(payloadObject);
    const output: Record<string, unknown> = isRecord(transformed) ? copyRecord(transformed) : Object.create(null);
    output[repr.tag] = variant.wire;
    return output;
  }
  if (repr.kind === "external") {
    const wire = variant.wire;
    if (typeof value === "string") return value;
    const output: Record<string, unknown> = Object.create(null);
    output[wire] = transformPayload(payloadValue);
    return output;
  }
  if (repr.kind === "adjacent") {
    const output: Record<string, unknown> = isRecord(value) ? copyRecord(value) : Object.create(null);
    output[repr.content] = transformPayload(payloadValue);
    return output;
  }
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

function defaultCookiePath(requestPath: string): string {
  const lastSlash = requestPath.lastIndexOf("/");
  return lastSlash <= 0 ? "/" : requestPath.slice(0, lastSlash);
}

function normalizeCookiePath(path: string | undefined, requestPath: string): string | undefined {
  if (path === undefined || path.length === 0) return undefined;
  return path.startsWith("/") ? path : defaultCookiePath(requestPath);
}

function removeCookieHeader(headers: Headers, name: string): void {
  const current = headers.get("cookie");
  if (current === null) return;
  const cookies = current
    .split(";")
    .map((part) => part.trim())
    .filter((part) => part.length > 0 && !part.startsWith(`${name}=`));
  if (cookies.length === 0) headers.delete("cookie");
  else headers.set("cookie", cookies.join("; "));
}

function cookiePathMatches(cookiePath: string, requestPath: string): boolean {
  if (!cookiePath || cookiePath === "/") return true;
  const prefix = cookiePath.endsWith("/") ? cookiePath : `${cookiePath}/`;
  return requestPath === cookiePath || requestPath.startsWith(prefix);
}

function setCookieHeader(headers: Headers, name: string, value: string, scheme: string): void {
  if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name) || /[;\u0000-\u001f\u007f]/.test(value)) {
    throw new CredentialSourceError(scheme, new Error("cookie credential contains an invalid character"));
  }
  const current = headers.get("cookie");
  const cookies = (current ?? "")
    .split(";")
    .map((part) => part.trim())
    .filter((part) => part.length > 0 && !part.startsWith(`${name}=`));
  cookies.push(`${name}=${value}`);
  headers.set("cookie", cookies.join("; "));
}

function cookieValue(header: string | null, name: string): { value: string; deleted: boolean; expiresAt?: number; path?: string; secure?: boolean; domain?: string } | undefined {
  const parts = header?.split(";").map((part) => part.trim()) ?? [];
  const cookie = parts.find((part) => part.startsWith(`${name}=`));
  if (cookie === undefined) return undefined;
  const maxAge = parts.slice(1).find((part) => part.toLowerCase().startsWith("max-age="));
  const expires = parts.slice(1).find((part) => part.toLowerCase().startsWith("expires="));
  const path = parts.slice(1).find((part) => part.toLowerCase().startsWith("path="));
  const domain = parts.slice(1).find((part) => part.toLowerCase().startsWith("domain="));
  const secure = parts.slice(1).some((part) => part.toLowerCase() === "secure");
  const maxAgeValue = maxAge === undefined ? undefined : Number(maxAge.slice("max-age=".length));
  const expiresValue = expires === undefined ? undefined : Date.parse(expires.slice("expires=".length));
  let deleted = false;
  let expiresAt: number | undefined;
  if (maxAgeValue !== undefined && Number.isFinite(maxAgeValue)) {
    deleted = maxAgeValue <= 0;
    if (!deleted) expiresAt = Date.now() + maxAgeValue * 1000;
  } else if (expiresValue !== undefined && Number.isFinite(expiresValue)) {
    deleted = expiresValue <= Date.now();
    if (!deleted) expiresAt = expiresValue;
  }
  return {
    value: cookie.slice(name.length + 1),
    deleted,
    expiresAt,
    path: path === undefined || path.slice("path=".length).trim().length === 0
      ? undefined
      : path.slice("path=".length).trim(),
    domain: domain === undefined ? undefined : domain.slice("domain=".length).trim(),
    secure,
  };
}

function pointerValue(value: unknown, pointer: string): unknown {
  if (pointer === "") return value;
  return pointer.split("/").slice(1).reduce<unknown>((current, part) => {
    const key = part.replaceAll("~1", "/").replaceAll("~0", "~");
    return current && typeof current === "object" && Object.hasOwn(current, key)
      ? (current as Record<string, unknown>)[key]
      : undefined;
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
