import type { RetryOptions } from "../../client.js";
import { findError } from "./status.js";

export const MAX_TIMER_DELAY = 2_147_483_647;

export async function fetchWithRetry(
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
