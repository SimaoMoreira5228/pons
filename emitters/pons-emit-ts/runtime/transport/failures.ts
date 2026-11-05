import type {
  AuthUnavailableError,
  DecodeError,
  RedirectError,
  RequestError,
} from "../../errors.js";
import type { ValidationIssue } from "../validate.js";

export class ResponseHeaderDecodeError extends Error {
  constructor(message: string, readonly sourceCause?: unknown) {
    super(message);
  }
}

export class RedirectFailure extends Error {}

export function authUnavailable(schemes: string[]): AuthUnavailableError {
  return { _tag: "AuthUnavailableError", schemes };
}

export function decodeFailure(status: number, issues: readonly string[], body: unknown, cause?: unknown): DecodeError {
  return cause === undefined
    ? { _tag: "DecodeError", status, issues, message: issues.join("; "), body }
    : { _tag: "DecodeError", status, issues, message: issues.join("; "), body, cause };
}

export function requestFailure(message: string): RequestError {
  return { _tag: "RequestError", message };
}

export function redirectFailure(message: string): RedirectError {
  return { _tag: "RedirectError", message };
}

export function contractFailure(violation: unknown, status: number, body: unknown): DecodeError {
  return decodeFailure(status, [contractMessage(violation)], body);
}

export function contractMessage(violation: unknown): string {
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
