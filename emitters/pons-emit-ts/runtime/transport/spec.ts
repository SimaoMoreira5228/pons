import type { AuthSchemeDefinition, CallOptions, ClientOptions } from "../../client.js";
import type { OpaqueContext } from "../codec.js";
import type { CredentialProvision, CredentialValue } from "../credentials.js";
import type { Validator } from "../validate.js";

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
export type HeaderDescriptor = { name: string; required: boolean; schema: unknown; validate?: Validator };
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
export type AttachedCustomCredential = { scheme: string; headers: string[]; query: Array<[string, string]>; cookies: string[] };
export type AttachedQueryCredential = { scheme: string; params: Array<[string, string]> };
export type AttachedCredentialOrigin = { scheme: string; origin: string };
export type AttachedCookie = { scheme: string; path?: string | undefined; secure?: boolean | undefined; host?: string | undefined };
