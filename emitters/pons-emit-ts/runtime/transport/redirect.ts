import type { RetryOptions } from "../../client.js";
import { parseJson } from "../codec.js";
import {
  CredentialRedirectFailure,
  captureCredential,
  cookiePathMatches,
  customAdapter,
  dropCredentialScheme,
  isCookieAuth,
  latestCredentialRecord,
  reattachQueryCredentials,
  removeCookieHeader,
  setCookieHeader,
  type CredentialProvision,
} from "../credentials.js";
import { RedirectFailure } from "./failures.js";
import { readBoundedBytes, readResponseText } from "./response-body.js";
import { fetchWithRetry } from "./retry.js";
import type {
  AttachedCookie,
  AttachedCredentialOrigin,
  AttachedCustomCredential,
  AttachedQueryCredential,
  ConfiguredClient,
} from "./spec.js";
import { declaresStatus } from "./status.js";

const MAX_REDIRECTS = 20;
const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);

export async function fetchWithRedirect(
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
