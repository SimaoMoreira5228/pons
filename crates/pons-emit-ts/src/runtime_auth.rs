pub(super) fn render() -> &'static str {
    r#"async function credentialValue(client: ConfiguredClient, scheme: string): Promise<string | undefined> {
  const expiration = client.authExpirations[scheme];
  if (expiration !== undefined && expiration <= Date.now()) {
    delete client.authValues[scheme];
    delete client.authExpirations[scheme];
    const source = client.auth?.[scheme];
    try {
      if (source !== undefined && typeof source === "object") await source.clear();
    } catch (cause) {
      throw new CredentialSourceError(scheme, cause);
    }
  }
  const cached = client.authValues[scheme];
  if (cached !== undefined) return cached;
  const source = client.auth?.[scheme];
  try {
    if (typeof source === "string") return source;
    if (typeof source === "function") return (await source()) ?? undefined;
    if (source !== undefined) return (await source.get()) ?? undefined;
  } catch (cause) {
    throw new CredentialSourceError(scheme, cause);
  }
  return undefined;
}

function readCredential(
  response: Response,
  locator: CredentialLocator,
  value: unknown,
  includeBody: boolean,
): unknown {
  if (locator.from === "body") {
    if (!includeBody) throw new Error("body credential locator cannot be used for a stream");
    return pointerValue(value, locator.pointer);
  }
  if (locator.from === "header") return response.headers.get(locator.name);
  return cookieValue(response.headers.get("set-cookie"), locator.name);
}

function parseExpiration(value: unknown): number {
  if (typeof value !== "string") throw new Error("credential expiry must be an ISO date-time");
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) throw new Error("credential expiry is not a valid date-time");
  return timestamp;
}

async function captureCredential(
  client: ConfiguredClient,
  response: Response,
  provides: CredentialProvision | undefined,
  value: unknown,
  includeBody: boolean,
): Promise<void> {
  if (!provides) return;
  const credential = readCredential(response, provides.value, value, includeBody);
  if (credential === undefined || credential === null) return;
  let expiresAt: number | undefined;
  if (provides.expiresAt) {
    const rawExpiration = readCredential(response, provides.expiresAt, value, includeBody);
    if (rawExpiration === undefined || rawExpiration === null) {
      throw new CredentialSourceError(provides.scheme, new Error("credential expiry is missing"));
    }
    try {
      expiresAt = parseExpiration(rawExpiration);
    } catch (cause) {
      throw new CredentialSourceError(provides.scheme, cause);
    }
  }
  const nextCredential = String(credential);
  const source = client.auth?.[provides.scheme];
  try {
    if (source !== undefined && typeof source === "object") {
      await source.set(nextCredential, expiresAt);
    }
  } catch (cause) {
    throw new CredentialSourceError(provides.scheme, cause);
  }
  client.authValues[provides.scheme] = nextCredential;
  if (expiresAt === undefined) delete client.authExpirations[provides.scheme];
  else client.authExpirations[provides.scheme] = expiresAt;
}
"#
}
