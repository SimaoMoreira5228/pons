import { MAX_STREAM_BUFFER } from "../stream.js";
import { WirePolicyError } from "../wire.js";

export async function readUnexpectedBody(response: Response, contentType: string): Promise<unknown> {
  if (response.status === 204 || response.status === 304 || !response.body) return undefined;
  const lowered = contentType.toLowerCase();
  if (lowered.includes("json") || lowered.startsWith("text/")) {
    return readResponseText(response, contentType);
  }
  return readBoundedBytes(response);
}

export async function responseHasBody(response: Response): Promise<boolean> {
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

export async function readBoundedBytes(response: Response): Promise<Uint8Array> {
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

export async function readResponseText(response: Response, contentType: string): Promise<string> {
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
