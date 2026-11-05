import {
  err,
  ok,
  type AbortedError,
  type ApiError,
  type AuthAdapterError,
  type AuthSourceError,
  type ClientFailure,
  type CredentialRedirectError,
  type NetworkError,
  type OpaqueCodecError,
  type Result,
  type TimeoutError,
  type TransportHookError,
  type UnexpectedStatusError,
} from "../../errors.js";
import {
  OpaqueCodecFailure,
  decodeWire,
  parseJson,
  stringifyCarrier,
  stringifyJson,
} from "../codec.js";
import {
  CredentialRedirectFailure,
  CredentialSourceError,
  captureCredential,
  cookiePathMatches,
  credentialRecords,
  credentialValue,
  customAdapter,
  isCookieAuth,
  removeCookieHeader,
  setCookieHeader,
  type CredentialValue,
} from "../credentials.js";
import { readNdjson, readSse } from "../stream.js";
import { WirePolicyError, hasOpaque } from "../wire.js";
import {
  RedirectFailure,
  ResponseHeaderDecodeError,
  authUnavailable,
  contractFailure,
  decodeFailure,
  redirectFailure,
  requestFailure,
} from "./failures.js";
import { preflightBody, preflightHeaders, preflightPath, preflightQuery } from "./preflight.js";
import { fetchWithRedirect } from "./redirect.js";
import {
  mediaTypeMatches,
  responseHeaders,
  responseMediaTypeMatches,
  withSuccessStatus,
} from "./response.js";
import {
  readBoundedBytes,
  readResponseText,
  readUnexpectedBody,
  responseHasBody,
} from "./response-body.js";
import { MAX_TIMER_DELAY } from "./retry.js";
import type {
  AttachedCookie,
  AttachedCredentialOrigin,
  AttachedCustomCredential,
  AttachedQueryCredential,
  ConfiguredClient,
  RequestSpec,
} from "./spec.js";
import { matchResponse } from "./status.js";

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

export async function requestOrThrow<T, E>(operation: Promise<Result<T, E | ClientFailure>>): Promise<T> {
  const result = await operation;
  if (result.isErr()) throw result.error;
  return result.value as T;
}
