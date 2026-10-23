import type { AuthSchemeDefinition, CallOptions, ClientOptions, RetryOptions } from "../client.js";
import {
  err,
  ok,
  type AbortedError,
  type ApiError,
  type AuthAdapterError,
  type AuthSourceError,
  type AuthUnavailableError,
  type ClientFailure,
  type CredentialRedirectError,
  type DecodeError,
  type NetworkError,
  type OpaqueCodecError,
  type RedirectError,
  type RequestError,
  type Result,
  type TimeoutError,
  type TransportHookError,
  type UnexpectedStatusError,
} from "../errors.js";
import { typeDefinitions } from "../types.js";
import {
  OpaqueCodecFailure,
  copyRecord,
  decodeWire,
  encodeWire,
  parseIntegerNumber,
  parseIntegerText,
  parseJson,
  parseNumber,
  stringifyCarrier,
  stringifyJson,
  type OpaqueContext,
} from "./codec.js";
import {
  CredentialRedirectFailure,
  CredentialSourceError,
  captureCredential,
  cookiePathMatches,
  credentialRecords,
  credentialValue,
  customAdapter,
  dropCredentialScheme,
  isCookieAuth,
  latestCredentialRecord,
  reattachQueryCredentials,
  removeCookieHeader,
  setCookieHeader,
  type CredentialProvision,
  type CredentialValue,
} from "./credentials.js";
import { MAX_STREAM_BUFFER, readNdjson, readSse } from "./stream.js";
import type { Validator, ValidationIssue } from "./validate.js";
import { WirePolicyError, definitionApplies, hasOpaque, validateWire } from "./wire.js";

const MAX_TIMER_DELAY = 2_147_483_647;
const MAX_REDIRECTS = 20;
const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);

class ResponseHeaderDecodeError extends Error {
  constructor(message: string, readonly sourceCause?: unknown) {
    super(message);
  }
}

class RedirectFailure extends Error {}

export type ConfiguredClient = ClientOptions & {
  authRecords: Record<string, CredentialValue[]>;
  authInvalidated: Record<string, boolean>;
  authSchemes: Record<string, AuthSchemeDefinition>;
  opaque: OpaqueContext;
};

type RequestInputDescriptor = {
  path: Array<{ name: string; required: boolean; schema: unknown }>;
  query?: unknown;
  headers: Array<{ name: string; required: boolean; schema: unknown }>;
  bodyRequired: boolean;
};
type HeaderDescriptor = { name: string; required: boolean; schema: unknown; validate?: Validator };
type ResponseDescriptor = {
  status: number | string;
  content: string;
  mediaType?: string;
  schema?: unknown;
  stream?: "sse" | "ndjson";
  events?: Array<{ event: string; schema: unknown; validate?: Validator }>;
  headers?: HeaderDescriptor[];
  validate?: Validator;
};
type ErrorDescriptor = {
  status: number | string;
  content: string;
  mediaType?: string;
  schema?: unknown;
  retryable?: boolean;
  events?: Array<{ event: string; schema: unknown; validate?: Validator }>;
  headers?: HeaderDescriptor[];
  validate?: Validator;
};
export type RequestSpec = {
  method: string;
  path: string | ((values: Record<string, unknown>) => string);
  pathValues?: Record<string, unknown> | undefined;
  query?: unknown;
  headers?: Record<string, unknown> | undefined;
  body?: unknown;
  bodyContent?: string | undefined;
  bodyMediaType?: string | undefined;
  bodySchema?: unknown;
  auth?: { alternatives?: string[][]; scopes?: Record<string, readonly string[]>; optional?: boolean } | undefined;
  inputs: RequestInputDescriptor;
  responses: ResponseDescriptor[];
  errors: ErrorDescriptor[];
  preserveWideIntegers?: boolean | undefined;
  provides?: CredentialProvision | undefined;
  options?: CallOptions | undefined;
  idempotent?: boolean | undefined;
};
type AttachedCustomCredential = { scheme: string; headers: string[]; query: Array<[string, string]>; cookies: string[] };
export type AttachedQueryCredential = { scheme: string; params: Array<[string, string]> };
type AttachedCredentialOrigin = { scheme: string; origin: string };
type AttachedCookie = { scheme: string; path?: string | undefined; secure?: boolean | undefined; host?: string | undefined };

function authUnavailable(schemes: string[]): AuthUnavailableError {
  return { _tag: "AuthUnavailableError", schemes };
}

function decodeFailure(status: number, issues: readonly string[], body: unknown, cause?: unknown): DecodeError {
  return cause === undefined
    ? { _tag: "DecodeError", status, issues, message: issues.join("; "), body }
    : { _tag: "DecodeError", status, issues, message: issues.join("; "), body, cause };
}

function requestFailure(message: string): RequestError {
  return { _tag: "RequestError", message };
}

function redirectFailure(message: string): RedirectError {
  return { _tag: "RedirectError", message };
}

export async function request<T, E>(
  client: ConfiguredClient,
  spec: RequestSpec,
): Promise<Result<T, E | ClientFailure>> {
  const { method, path, inputs, responses, errors } = spec;
  const { body, bodyContent: content, bodyMediaType: mediaType, bodySchema } = spec;
  const pathValues = spec.pathValues;
  const query = spec.query;
  const requestHeaders = spec.headers;
  const authGroups = spec.auth?.alternatives ?? [];
  const authScopes = spec.auth?.scopes ?? {};
  const authOptional = spec.auth?.optional === true;
  const preserveWideIntegers = spec.preserveWideIntegers === true;
  const provides = spec.provides;
  const callOptions = spec.options;
  const idempotent = spec.idempotent === true;
  const configuredTimeout = callOptions?.timeoutMs ?? client.timeoutMs;
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
  let phase: "request" | "response" = "request";
  let responseStatus = 0;
  let responseBody: unknown;
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
      return err(authUnavailable(authGroups.flat()));
    }
    const attachedCredentialOrigins: AttachedCredentialOrigin[] = [];
    const attachedCookies: AttachedCookie[] = [];
    const attachedCustomCredentials: AttachedCustomCredential[] = [];
    const attachedQueryCredentials: AttachedQueryCredential[] = [];
    for (const scheme of activeGroup ?? []) {
      const definition = client.authSchemes[scheme];
      if (definition === undefined) continue;
      if (definition.kind === "custom") {
        const adapter = customAdapter(client, definition, scheme);
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
          return err({ _tag: "AuthAdapterError", scheme, cause } satisfies AuthAdapterError);
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
        if (!authOptional) return err(authUnavailable([scheme]));
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
    const init: RequestInit = { method, headers, redirect: "manual" };
    if (credentials !== undefined) init.credentials = credentials;
    if (signal !== undefined) init.signal = signal;
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
    if (client.transport?.onRequest) {
      try {
        await client.transport.onRequest({ url, init });
      } catch (cause) {
        return err({ _tag: "TransportHookError", phase: "request", cause } satisfies TransportHookError);
      }
    }
    init.redirect = "manual";
    for (const credential of attachedCredentialOrigins) {
      if (credential.origin !== url.origin) {
        return err(authUnavailable([credential.scheme]));
      }
    }
    for (const cookie of attachedCookies) {
      const definition = client.authSchemes[cookie.scheme];
      if (definition === undefined || !isCookieAuth(definition)) continue;
      if (
        (cookie.host !== undefined && cookie.host !== url.hostname) ||
        (cookie.path !== undefined && !cookiePathMatches(cookie.path, url.pathname)) ||
        (cookie.secure && url.protocol !== "https:")
      ) {
        removeCookieHeader(headers, definition.name!);
        return err(authUnavailable([cookie.scheme]));
      }
    }
    phase = "response";
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
    responseStatus = response.status;
    if (client.transport?.onResponse) {
      try {
        await client.transport.onResponse({ response, url: finalUrl, init: finalInit });
      } catch (cause) {
        return err({ _tag: "TransportHookError", phase: "response", cause } satisfies TransportHookError);
      }
    }
    const responseContent = response.headers.get("content-type") ?? "";
    const matched = matchResponse(responses, errors, response.status);
    const declaredSuccess = matched.success;
    const declaredError = matched.error;
    if (declaredSuccess === undefined && declaredError === undefined) {
      return err({
        _tag: "UnexpectedStatusError",
        status: response.status,
        body: await readUnexpectedBody(response, responseContent),
      } satisfies UnexpectedStatusError);
    }
    const responseSchema = declaredSuccess ?? declaredError;
    const missingHeader = responseSchema?.headers?.find((header) => header.required && !response.headers.has(header.name));
    if (missingHeader) {
      return err(decodeFailure(response.status, [`missing required response header ${missingHeader.name}`], undefined));
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
      return err(decodeFailure(response.status, [`expected ${expectedContentType}, received ${responseContent || "no content type"}`], undefined));
    }
    if (declaredSuccess?.content === "stream") {
      if (!response.body) return err(decodeFailure(response.status, ["the event stream has no response body"], undefined));
      await captureCredential(client, response, provides, undefined, false, finalUrl);
      if (declaredSuccess.stream === "ndjson") {
        return ok(withSuccessStatus(readNdjson(response.body, declaredSuccess.schema, declaredSuccess.validate, preserveWideIntegers, client.opaque), response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
      }
      return ok(withSuccessStatus(readSse(response.body, declaredSuccess.events ?? [], preserveWideIntegers, client.opaque), response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
    }
    const jsonResponse = responseKind === "json" || responseKind === "problem_json" ||
      (!responseKind && responseContent.includes("json"));
    const textResponse = responseKind === "text" ||
      (!responseKind && responseContent.startsWith("text/"));
    if (responseKind === "empty" && await responseHasBody(response)) {
      return err(decodeFailure(response.status, ["empty response contains a body"], undefined));
    }
    let rawValue: unknown;
    if (response.status === 204 || response.status === 304 || responseKind === "empty") {
      rawValue = undefined;
    } else if (jsonResponse) {
      const text = await readResponseText(response, responseContent);
      try {
        rawValue = parseJson(text, preserveWideIntegers);
      } catch (cause) {
        return err(decodeFailure(response.status, ["invalid JSON response"], text, cause));
      }
    } else if (textResponse) {
      rawValue = await readResponseText(response, responseContent);
    } else {
      rawValue = await readBoundedBytes(response);
    }
    responseBody = rawValue;
    if (declaredSuccess !== undefined) {
      await captureCredential(client, response, provides, rawValue, true, finalUrl);
    }
    const schema = responseSchema?.schema;
    const decoded = responseKind !== "bytes"
      && Boolean(schema && (jsonResponse || textResponse || hasOpaque(schema)));
    const value = decoded ? decodeWire(rawValue, schema, preserveWideIntegers, client.opaque) : rawValue;
    if (responseSchema?.validate && (decoded || responseKind === "bytes")) {
      let violation: unknown;
      try {
        violation = responseSchema.validate(value);
      } catch (cause) {
        return err(decodeFailure(
          response.status,
          ["the response does not match the contract: the validator threw"],
          rawValue,
          cause,
        ));
      }
      if (violation !== null) return err(contractFailure(violation, response.status, rawValue));
    }
    if (declaredSuccess === undefined) {
      return declaredError
        ? err({
          _tag: "ApiError",
          status: response.status,
          body: value,
          headers: declaredError.headers
            ? responseHeaders(response, declaredError.headers, preserveWideIntegers, client.opaque)
            : response.headers,
        } satisfies ApiError<number, unknown, unknown> as E)
        : err({ _tag: "UnexpectedStatusError", status: response.status, body: value } satisfies UnexpectedStatusError);
    }
    return ok(withSuccessStatus(value, response, declaredSuccess, preserveWideIntegers, client.opaque) as T);
  } catch (cause) {
    if (timedOut) return err({ _tag: "TimeoutError", afterMs: timeoutMs! } satisfies TimeoutError);
    if (cause instanceof CredentialRedirectFailure) {
      return err(cause.schemes.length > 0
        ? { _tag: "CredentialRedirectError", schemes: cause.schemes } satisfies CredentialRedirectError
        : redirectFailure(cause.message));
    }
    if (cause instanceof RedirectFailure) return err(redirectFailure(cause.message));
    if (cause instanceof ResponseHeaderDecodeError) {
      return err(decodeFailure(responseStatus, [cause.message], responseBody, cause.sourceCause));
    }
    if (cause instanceof WirePolicyError) {
      return err(phase === "request"
        ? requestFailure(cause.message)
        : decodeFailure(responseStatus, [cause.message], responseBody));
    }
    if (cause instanceof CredentialSourceError) {
      return err({ _tag: "AuthSourceError", scheme: cause.scheme, cause: cause.sourceCause } satisfies AuthSourceError);
    }
    if (cause instanceof OpaqueCodecFailure) {
      return err({ _tag: "OpaqueCodecError", codec: cause.codec, phase: cause.phase, cause: cause.sourceCause } satisfies OpaqueCodecError);
    }
    if (typeof DOMException !== "undefined" && cause instanceof DOMException && cause.name === "AbortError") {
      return err({ _tag: "AbortedError", cause } satisfies AbortedError);
    }
    return err({ _tag: "NetworkError", cause } satisfies NetworkError);
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
  const forced = options?.force === true;
  const attempts = (idempotent || forced) && Number.isFinite(configuredAttempts)
    ? Math.max(1, Math.floor(configuredAttempts))
    : 1;
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
    const declaredError = response.status >= 400 ? findError(errors, response.status) : undefined;
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
  attachedCredentialOrigins: AttachedCredentialOrigin[],
  attachedCookies: AttachedCookie[],
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
        throw new RedirectFailure("invalid redirect location");
      }
      if ((nextUrl.protocol !== "http:" && nextUrl.protocol !== "https:") || nextUrl.username !== "" || nextUrl.password !== "") {
        throw new RedirectFailure("redirect target is not an HTTP origin");
      }
      if (visited.has(nextUrl.href)) throw new RedirectFailure("redirect loop");
      if (redirectCount >= MAX_REDIRECTS) throw new RedirectFailure("too many redirects");
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
  attachedCredentialOrigins: AttachedCredentialOrigin[],
  attachedCookies: AttachedCookie[],
  attachedCustomCredentials: AttachedCustomCredential[],
): void {
  const originChanged = nextUrl.origin !== currentUrl.origin;
  const wrongOriginSchemes = attachedCredentialOrigins
    .filter((credential) => credential.origin !== nextUrl.origin)
    .map((credential) => credential.scheme);
  if (originChanged && wrongOriginSchemes.length > 0) throw new CredentialRedirectFailure(wrongOriginSchemes);
  for (const cookie of attachedCookies) {
    if (
      (cookie.host !== undefined && cookie.host !== nextUrl.hostname) ||
      (cookie.path !== undefined && !cookiePathMatches(cookie.path, nextUrl.pathname)) ||
      (cookie.secure && nextUrl.protocol !== "https:")
    ) {
      throw new CredentialRedirectFailure([cookie.scheme]);
    }
  }
  if (!originChanged) return;
  if (init.body !== undefined && init.body !== null) throw new CredentialRedirectFailure([]);
  if (nextUrl.search !== "") throw new CredentialRedirectFailure([]);
  if (credentials === "include") throw new CredentialRedirectFailure([]);
  const headers = new Headers(init.headers);
  if (headers.has("authorization") || headers.has("cookie")) throw new CredentialRedirectFailure([]);
  for (const credential of attachedCustomCredentials) {
    if (credential.headers.some((name) => headers.has(name))) throw new CredentialRedirectFailure([credential.scheme]);
  }
}

async function applyCapturedCredential(
  client: ConfiguredClient,
  provides: CredentialProvision | undefined,
  url: URL,
  init: RequestInit,
  method: string,
  authScopes: Record<string, readonly string[]>,
  attachedCookies: AttachedCookie[],
  attachedCustomCredentials: AttachedCustomCredential[],
  attachedQueryCredentials: AttachedQueryCredential[],
): Promise<RequestInit> {
  if (provides === undefined) return init;
  const definition = client.authSchemes[provides.scheme];
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
      const adapter = customAdapter(client, definition, provides.scheme);
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
        throw new CredentialRedirectFailure([provides.scheme]);
      }
      setCookieHeader(headers, name, value, provides.scheme);
      const scope = { scheme: provides.scheme, path, secure, host };
      const existing = attachedCookies[cookieIndex];
      if (existing !== undefined) Object.assign(existing, scope);
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

export async function requestOrThrow<T, E>(operation: Promise<Result<T, E | ClientFailure>>): Promise<T> {
  const result = await operation;
  if (result.isErr()) throw result.error;
  return result.value as T;
}

export function encodePathSegment(value: string): string {
  if (value === "." || value === "..") throw new WirePolicyError("path segment cannot be a dot segment");
  return encodeURIComponent(value);
}

function mediaTypeBase(value: string): string {
  return (value.split(";", 1)[0] ?? "").trim().toLowerCase();
}

function responseMediaTypeMatches(actual: string, expected: string): boolean {
  return mediaTypeBase(actual) === mediaTypeBase(expected);
}

function normalizeMediaParameter(part: string): [string, string] {
  const [key, ...value] = part.split("=");
  return [(key ?? "").trim().toLowerCase(), value.join("=").trim().toLowerCase()];
}

function mediaTypeMatches(actual: string, expected: string): boolean {
  if (mediaTypeBase(actual) !== mediaTypeBase(expected)) return false;
  const actualParameters = new Map(actual.split(";").slice(1).map(normalizeMediaParameter));
  return expected.split(";").slice(1).every((part) => {
    const [key, value] = normalizeMediaParameter(part);
    return actualParameters.get(key) === value;
  });
}

export function preflightPath(
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

export function preflightQuery(value: unknown, schema: unknown, opaque: OpaqueContext): unknown {
  if (schema == null) return value;
  if (value === undefined) throw new WirePolicyError("missing request query");
  return encodeWire(value, schema, opaque);
}

export function preflightHeaders(
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
  return findSpecific(entries, actual) !== undefined;
}

function findSpecific<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`);
}

function findCatchAll<T extends { status: number | string }>(entries: T[]): T | undefined {
  return entries.find((entry) => entry.status === "unknown")
    ?? entries.find((entry) => entry.status === "default");
}

function findError<E extends { status: number | string }>(entries: E[], actual: number): E | undefined {
  return findSpecific(entries, actual) ?? findCatchAll(entries);
}

function contractFailure(violation: unknown, status: number, body: unknown): DecodeError {
  return decodeFailure(status, [contractMessage(violation)], body);
}

function contractMessage(violation: unknown): string {
  if (
    typeof violation !== "object"
    || violation === null
    || !Array.isArray((violation as { path?: unknown }).path)
    || typeof (violation as { message?: unknown }).message !== "string"
  ) {
    return "the response does not match the contract: the validator returned an invalid issue";
  }
  const { path, message } = violation as ValidationIssue;
  const location = path.length === 0 ? "" : `${contractPath(path)}: `;
  return `the response does not match the contract: ${location}${message}`;
}

function contractPath(path: readonly (string | number)[]): string {
  let output = "";
  for (const step of path) {
    if (typeof step === "number") {
      output += `[${step}]`;
    } else {
      output += output === "" ? step : `.${step}`;
    }
  }
  return output;
}

function matchResponse<T extends { status: number | string }, E extends { status: number | string }>(
  responses: T[],
  errors: E[],
  actual: number,
): { success: T | undefined; error: E | undefined } {
  const success = findSpecific(responses, actual);
  if (success !== undefined) return { success, error: undefined };
  const error = findError(errors, actual);
  if (error !== undefined) return { success: undefined, error };
  return { success: findCatchAll(responses), error: undefined };
}

function withSuccessStatus(
  value: unknown,
  response: Response,
  schema: { status: number | string; headers?: HeaderDescriptor[] } | undefined,
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
  schema: { headers?: HeaderDescriptor[] } | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): unknown {
  if (!schema?.headers) return value;
  return { value, headers: responseHeaders(response, schema.headers, preserveWideIntegers, opaque) };
}

function responseHeaders(
  response: Response,
  headers: HeaderDescriptor[],
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const values: Record<string, unknown> = Object.create(null);
  for (const header of headers) {
    const headerValue = response.headers.get(header.name);
    if (headerValue === null) continue;
    const decoded = decodeHeaderValue(headerValue, header.schema, preserveWideIntegers, opaque);
    if (header.validate) {
      let violation: unknown;
      try {
        violation = header.validate(decoded);
      } catch (cause) {
        throw new ResponseHeaderDecodeError("the response does not match the contract: the validator threw", cause);
      }
      if (violation !== null) throw new ResponseHeaderDecodeError(contractMessage(violation));
    }
    values[header.name] = decoded;
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
    if (cause instanceof ResponseHeaderDecodeError || cause instanceof OpaqueCodecFailure) throw cause;
    throw new ResponseHeaderDecodeError(`invalid response header value ${JSON.stringify(value)}`);
  }
}

async function readUnexpectedBody(response: Response, contentType: string): Promise<unknown> {
  if (response.status === 204 || response.status === 304 || !response.body) return undefined;
  const lowered = contentType.toLowerCase();
  if (lowered.includes("json") || lowered.startsWith("text/")) {
    return readResponseText(response, contentType);
  }
  return readBoundedBytes(response);
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
