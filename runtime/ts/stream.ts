import { err, ok, type NdjsonStreamError, type Result, type SseStreamError } from "../errors.js";
import { decodeWire, parseJson, type OpaqueContext } from "./codec.js";
import type { Validator } from "./validate.js";
import { WirePolicyError } from "./wire.js";

export const MAX_STREAM_BUFFER = 8 * 1024 * 1024;
const MAX_STREAM_FIELDS = 100_000;

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

export async function* readSse<EventValue, ErrorBody = unknown>(
  body: ReadableStream<Uint8Array>,
  events: Array<{ event: string; schema: unknown; validate?: Validator }>,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): AsyncIterable<Result<EventValue, SseStreamError<ErrorBody>>> {
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
              yield err({ _tag: "StreamUnknownEventError", event: name });
            } else {
              try {
                const parsed = parseJson(data.join("\n"), preserveWideIntegers);
                const decoded = decodeWire(parsed, descriptor.schema, preserveWideIntegers, opaque);
                const violation = descriptor.validate === undefined ? null : descriptor.validate(decoded);
                if (violation !== null) yield err({ _tag: "StreamDecodeError", event: name, cause: violation });
                else if (name === "error") yield err({ _tag: "StreamServerError", body: decoded as ErrorBody });
                else yield ok({ event: name, data: decoded } as EventValue);
              } catch (cause) {
                yield err({ _tag: "StreamDecodeError", event: name, cause });
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
    yield err({ _tag: "StreamReadError", cause });
  } finally {
    try {
      await reader.cancel();
    } catch {}
    reader.releaseLock();
  }
}

export async function* readNdjson<Item>(
  body: ReadableStream<Uint8Array>,
  schema: unknown,
  validate: Validator | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): AsyncIterable<Result<Item, NdjsonStreamError>> {
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
            const decoded = decodeWire(parsed, schema, preserveWideIntegers, opaque) as Item;
            const violation = validate === undefined ? null : validate(decoded);
            if (violation !== null) yield err({ _tag: "StreamDecodeError", cause: violation });
            else yield ok(decoded);
          } catch (cause) {
            yield err({ _tag: "StreamDecodeError", cause });
          }
        }
        lineEnd = nextLineEnd(buffer, done);
      }
      if (done) break;
    }
  } catch (cause) {
    yield err({ _tag: "StreamReadError", cause });
  } finally {
    try {
      await reader.cancel();
    } catch {}
    reader.releaseLock();
  }
}
