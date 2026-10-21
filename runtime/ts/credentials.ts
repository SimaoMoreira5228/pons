import type { AuthAdapter, AuthSchemeDefinition, CredentialMetadata, StoredCredential, TokenStore } from "../client.js";
import type { AttachedQueryCredential, ConfiguredClient } from "./transport.js";

export type TokenStorage = {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
};

type CredentialLocator =
  | { from: "body"; pointer: string }
  | { from: "header"; name: string }
  | { from: "set_cookie"; name: string };
export type CredentialProvision = { scheme: string; value: CredentialLocator; expiresAt?: CredentialLocator | undefined };
export type CredentialValue = { value: string | undefined; expiresAt?: number | undefined; path?: string | undefined; secure?: boolean | undefined; host?: string | undefined; origin?: string | undefined };

type CredentialRead = { value: unknown; deleted: boolean; expiresAt?: number | undefined; path?: string | undefined; secure?: boolean | undefined; domain?: string | undefined };

const CREDENTIAL_METADATA_KINDS: ReadonlyArray<readonly [keyof CredentialMetadata, string]> = [["expiresAt", "number"], ["path", "string"], ["secure", "boolean"], ["host", "string"], ["origin", "string"]];

function storedCredential(stored: unknown): StoredCredential | undefined {
  if (stored === undefined) return undefined;
  if (typeof stored === "string") {
    throw new Error("token store must return a credential record, not a string");
  }
  if (typeof stored !== "object" || stored === null) {
    throw new Error("token store must return a credential record");
  }
  const record = stored as StoredCredential;
  if (typeof record.value !== "string") {
    throw new Error("token store record must have a string value");
  }
  for (const [key, kind] of CREDENTIAL_METADATA_KINDS) {
    const value = record[key];
    if (value === undefined) continue;
    if (typeof value !== kind || (kind === "number" && Number.isNaN(value))) {
      throw new Error(`token store record ${key} must be a ${kind}`);
    }
  }
  return record;
}

export type StorageTokenStoreOptions = { key?: string; storage?: TokenStorage | undefined };

const DEFAULT_TOKEN_KEY = "pons.token";

function storageTokenStore(
  name: "localStorage" | "sessionStorage",
  options: StorageTokenStoreOptions,
): TokenStore {
  const storage = options.storage
    ?? (globalThis as { localStorage?: TokenStorage; sessionStorage?: TokenStorage })[name];
  if (storage === undefined) throw new Error(`${name} is not available in this environment`);
  const key = options.key ?? DEFAULT_TOKEN_KEY;
  return {
    get() {
      const raw = storage.getItem(key);
      if (raw === null) return undefined;
      let record: unknown;
      try {
        record = JSON.parse(raw);
      } catch (cause) {
        throw new Error(`the ${name} record for ${key} is not valid JSON`, { cause });
      }
      return storedCredential(record);
    },
    set(value, metadata) {
      const record: StoredCredential = metadata === undefined ? { value } : { value, ...metadata };
      storage.setItem(key, JSON.stringify(record));
    },
    clear() {
      storage.removeItem(key);
    },
  };
}

export function localStorageTokenStore(options: StorageTokenStoreOptions = {}): TokenStore {
  return storageTokenStore("localStorage", options);
}

export function sessionStorageTokenStore(options: StorageTokenStoreOptions = {}): TokenStore {
  return storageTokenStore("sessionStorage", options);
}

export function credentialRecords(client: ConfiguredClient, scheme: string): CredentialValue[] {
  return client.authRecords[scheme] ?? [];
}

export function latestCredentialRecord(client: ConfiguredClient, scheme: string): CredentialValue | undefined {
  const records = credentialRecords(client, scheme);
  return records.length === 0 ? undefined : records[records.length - 1];
}

function setCredentialRecords(client: ConfiguredClient, scheme: string, records: CredentialValue[]): void {
  if (records.length === 0) delete client.authRecords[scheme];
  else client.authRecords[scheme] = records;
}

function credentialScopeKey(record: CredentialValue): string {
  return JSON.stringify([record.host, record.path, record.secure === true, record.origin]);
}

function storeCredentialRecord(client: ConfiguredClient, scheme: string, record: CredentialValue): void {
  const key = credentialScopeKey(record);
  setCredentialRecords(client, scheme, [...credentialRecords(client, scheme).filter((item) => credentialScopeKey(item) !== key), record]);
}

function credentialApplies(record: CredentialValue, url: URL): boolean {
  return (record.origin === undefined || record.origin === url.origin) &&
    (record.host === undefined || record.host === url.hostname) &&
    (record.secure !== true || url.protocol === "https:") &&
    (record.path === undefined || cookiePathMatches(record.path, url.pathname));
}

function selectCredentialRecord(client: ConfiguredClient, scheme: string, url: URL): CredentialValue | undefined {
  let selected: CredentialValue | undefined;
  for (const record of credentialRecords(client, scheme)) {
    if (!credentialApplies(record, url)) continue;
    if (selected === undefined || (record.path?.length ?? 0) > (selected.path?.length ?? 0)) selected = record;
  }
  return selected;
}

async function expireCredentialRecords(client: ConfiguredClient, scheme: string): Promise<void> {
  const records = credentialRecords(client, scheme);
  const live = records.filter((record) => record.expiresAt === undefined || record.expiresAt > Date.now());
  if (live.length === records.length) return;
  setCredentialRecords(client, scheme, live);
  if (live.length > 0) return;
  const source = client.auth?.[scheme];
  if (source === undefined || typeof source !== "object") client.authInvalidated[scheme] = true;
  try {
    if (source !== undefined && typeof source === "object") await source.clear();
  } catch (cause) {
    throw new CredentialSourceError(scheme, cause);
  }
}

export async function credentialValue(client: ConfiguredClient, scheme: string, url: URL): Promise<CredentialValue> {
  await expireCredentialRecords(client, scheme);
  if (client.authInvalidated[scheme] === true) return { value: undefined };
  const cached = selectCredentialRecord(client, scheme, url);
  if (cached !== undefined) return cached;
  const source = client.auth?.[scheme];
  try {
    if (typeof source === "string") return { value: source };
    if (typeof source === "function") return { value: (await source()) ?? undefined };
    if (source !== undefined) {
      const stored = storedCredential(await source.get());
      if (stored !== undefined) {
        if (isCookieAuth(client.authSchemes[scheme]) && (stored.host === undefined || stored.path === undefined)) {
          throw new Error("cookie token store record is missing scope metadata");
        }
        if (stored.expiresAt !== undefined && stored.expiresAt <= Date.now()) {
          await source.clear();
          return { value: undefined };
        }
        return stored;
      }
    }
  } catch (cause) {
    throw new CredentialSourceError(scheme, cause);
  }
  return { value: undefined };
}

export class CredentialSourceError extends Error {
  constructor(readonly scheme: string, readonly sourceCause: unknown) {
    super(`credential source failed for ${scheme}`);
  }
}

export class CredentialRedirectError extends Error {
  constructor(readonly schemes: string[]) {
    super(schemes.length > 0
      ? "redirect would forward credentials outside their scope"
      : "cross-origin redirect would carry request state outside the origin that produced it");
  }
}

function readCredential(
  response: Response,
  locator: CredentialLocator,
  value: unknown,
  includeBody: boolean,
  scheme: string,
): CredentialRead {
  if (locator.from === "body") {
    if (!includeBody) throw new CredentialSourceError(scheme, new Error("body credential locator cannot be used for a stream"));
    return { value: pointerValue(value, locator.pointer), deleted: false };
  }
  if (locator.from === "header") return { value: response.headers.get(locator.name), deleted: false };
  const headers = response.headers as Headers & { getSetCookie?: () => string[] };
  const combined = response.headers.get("set-cookie");
  const setCookies = typeof headers.getSetCookie === "function"
    ? headers.getSetCookie()
    : combined === null
      ? []
      : combined.split(/,(?=\s*[^;,=\s]+=[^;,]*)/).map((header) => header.trim()).filter((header) => header.length > 0);
  return [...setCookies].reverse().map((header) => cookieValue(header, locator.name)).find((value) => value !== undefined)
    ?? { value: undefined, deleted: false };
}

function parseExpiration(value: unknown): number {
  if (typeof value !== "string") throw new Error("credential expiry must be an ISO date-time");
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) throw new Error("credential expiry is not a valid date-time");
  return timestamp;
}

function responseRequestUrl(response: Response, fallback: URL): URL {
  if (response.url.length === 0) return fallback;
  try {
    const responseUrl = new URL(response.url);
    return responseUrl.protocol === "http:" || responseUrl.protocol === "https:" ? responseUrl : fallback;
  } catch {
    return fallback;
  }
}

function cookieScopeMatches(
  expectedHost: string | undefined,
  expectedPath: string | undefined,
  expectedSecure: boolean | undefined,
  responseHost: string,
  responsePath: string,
  responseProtocol: string,
): boolean {
  return (expectedHost === undefined || expectedHost === responseHost) &&
    (expectedPath === undefined || expectedPath === responsePath) &&
    (expectedSecure !== true || responseProtocol === "https:");
}

export async function captureCredential(
  client: ConfiguredClient,
  response: Response,
  provides: CredentialProvision | undefined,
  value: unknown,
  includeBody: boolean,
  requestUrl: URL,
): Promise<void> {
  if (!provides) return;
  const credentialUrl = responseRequestUrl(response, requestUrl);
  const credentialRead = readCredential(response, provides.value, value, includeBody, provides.scheme);
  if (credentialRead.value === undefined || credentialRead.value === null) return;
  const credentialPath = provides.value.from === "set_cookie"
    ? normalizeCookiePath(credentialRead.path, credentialUrl.pathname) ?? defaultCookiePath(credentialUrl.pathname)
    : undefined;
  if (provides.value.from === "set_cookie") {
    const definition = client.authSchemes[provides.scheme];
    const targetName = isCookieAuth(definition) ? definition?.name : undefined;
    const prefixNames = targetName === undefined ? [provides.value.name] : [provides.value.name, targetName];
    const invalidPrefix = prefixNames.some((name) =>
      (name.startsWith("__Secure-") && !credentialRead.secure) ||
      (name.startsWith("__Host-") && (!credentialRead.secure || credentialPath !== "/" || credentialRead.domain !== undefined)));
    if (invalidPrefix) {
      if (credentialRecords(client, provides.scheme).length === 0 && client.auth?.[provides.scheme] === undefined) {
        client.authInvalidated[provides.scheme] = true;
      }
      return;
    }
  }
  if (credentialRead.deleted) {
    if (client.authInvalidated[provides.scheme] === true) return;
    const responsePath = credentialPath ?? defaultCookiePath(credentialUrl.pathname);
    const cached = credentialRecords(client, provides.scheme);
    const remaining = cached.filter((record) => !cookieScopeMatches(
      record.host,
      record.path,
      record.secure,
      credentialUrl.hostname,
      responsePath,
      credentialUrl.protocol,
    ));
    if (remaining.length !== cached.length) setCredentialRecords(client, provides.scheme, remaining);
    else if (cached.length > 0) return;
    const source = client.auth?.[provides.scheme];
    if (cached.length === 0 && source !== undefined && typeof source === "object") {
      let stored: StoredCredential | undefined;
      try {
        stored = storedCredential(await source.get());
      } catch (cause) {
        throw new CredentialSourceError(provides.scheme, cause);
      }
      if (stored === undefined) return;
      if (isCookieAuth(client.authSchemes[provides.scheme]) && (stored.host === undefined || stored.path === undefined)) {
        throw new CredentialSourceError(provides.scheme, new Error("cookie token store record is missing scope metadata"));
      }
      if (!cookieScopeMatches(
        stored.host,
        stored.path,
        stored.secure,
        credentialUrl.hostname,
        responsePath,
        credentialUrl.protocol,
      )) return;
    }
    if (credentialRecords(client, provides.scheme).length === 0) client.authInvalidated[provides.scheme] = true;
    try {
      if (source !== undefined && typeof source === "object") await source.clear();
    } catch (cause) {
      throw new CredentialSourceError(provides.scheme, cause);
    }
    return;
  }
  if (provides.value.from === "set_cookie" && credentialRead.secure && credentialUrl.protocol !== "https:") {
    if (credentialRecords(client, provides.scheme).length === 0 && client.auth?.[provides.scheme] === undefined) {
      storeCredentialRecord(client, provides.scheme, { value: undefined, secure: true, path: credentialPath, host: credentialUrl.hostname, origin: credentialUrl.origin });
    }
    return;
  }
  let expiresAt = credentialRead.expiresAt;
  if (provides.expiresAt) {
    const expirationRead = readCredential(response, provides.expiresAt, value, includeBody, provides.scheme);
    if (expirationRead.value === undefined || expirationRead.value === null) {
      throw new CredentialSourceError(provides.scheme, new Error("credential expiry is missing"));
    }
    try {
      expiresAt = parseExpiration(expirationRead.value);
    } catch (cause) {
      throw new CredentialSourceError(provides.scheme, cause);
    }
  }
  const nextCredential = String(credentialRead.value);
  const source = client.auth?.[provides.scheme];
  try {
    if (source !== undefined && typeof source === "object") {
      await source.set(nextCredential, {
        expiresAt,
        path: credentialPath,
        secure: credentialRead.secure,
        host: credentialUrl.hostname,
        origin: credentialUrl.origin,
      });
    }
  } catch (cause) {
    throw new CredentialSourceError(provides.scheme, cause);
  }
  delete client.authInvalidated[provides.scheme];
  const next: CredentialValue = { value: nextCredential };
  if (expiresAt !== undefined) next.expiresAt = expiresAt;
  if (credentialPath !== undefined) next.path = credentialPath;
  if (credentialRead.secure) next.secure = true;
  next.host = credentialUrl.hostname;
  next.origin = credentialUrl.origin;
  storeCredentialRecord(client, provides.scheme, next);
}

export function isCookieAuth(definition: AuthSchemeDefinition | undefined): boolean {
  return definition?.kind === "cookie" || (definition?.kind === "api_key" && definition.location === "cookie");
}

export function customAdapter(
  client: ConfiguredClient,
  definition: AuthSchemeDefinition,
  scheme: string,
): AuthAdapter | undefined {
  return client.authAdapters?.[definition.adapter ?? scheme];
}

export function dropCredentialScheme<T extends { scheme: string }>(credentials: T[], scheme: string): void {
  for (let index = credentials.length - 1; index >= 0; index -= 1) {
    const credential = credentials[index];
    if (credential !== undefined && credential.scheme === scheme) credentials.splice(index, 1);
  }
}

export function reattachQueryCredentials(
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

function defaultCookiePath(requestPath: string): string {
  const lastSlash = requestPath.lastIndexOf("/");
  return lastSlash <= 0 ? "/" : requestPath.slice(0, lastSlash);
}

function normalizeCookiePath(path: string | undefined, requestPath: string): string | undefined {
  if (path === undefined || path.length === 0) return undefined;
  return path.startsWith("/") ? path : defaultCookiePath(requestPath);
}

export function removeCookieHeader(headers: Headers, name: string): void {
  const current = headers.get("cookie");
  if (current === null) return;
  const cookies = current
    .split(";")
    .map((part) => part.trim())
    .filter((part) => part.length > 0 && !part.startsWith(`${name}=`));
  if (cookies.length === 0) headers.delete("cookie");
  else headers.set("cookie", cookies.join("; "));
}

export function cookiePathMatches(cookiePath: string, requestPath: string): boolean {
  if (!cookiePath || cookiePath === "/") return true;
  const prefix = cookiePath.endsWith("/") ? cookiePath : `${cookiePath}/`;
  return requestPath === cookiePath || requestPath.startsWith(prefix);
}

export function setCookieHeader(headers: Headers, name: string, value: string, scheme: string): void {
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

function cookieValue(header: string | null, name: string): { value: string; deleted: boolean; expiresAt?: number | undefined; path?: string | undefined; secure?: boolean | undefined; domain?: string | undefined } | undefined {
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
