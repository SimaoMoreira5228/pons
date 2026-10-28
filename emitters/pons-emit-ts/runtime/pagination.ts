import { err, ok, type Result } from "../errors.js";

export type PaginationCarrier = "query" | "body";

export type PaginationOverrides = {
  query: Record<string, unknown>;
  body: Record<string, unknown>;
};

export type PaginationDescriptor = {
  kind: "cursor" | "offset";
  field: string;
  carrier: PaginationCarrier;
  items: string;
  next?: string | undefined;
  initial?: unknown;
  pageSize?: {
    field: string;
    carrier: PaginationCarrier;
    default?: number | undefined;
    initial?: unknown;
  } | undefined;
  wrap: { status: boolean; headers: boolean };
};

type PageBody = { value?: unknown };

function pointerSegments(pointer: string): string[] | undefined {
  if (pointer === "") return [];
  if (!pointer.startsWith("/")) return undefined;
  return pointer
    .slice(1)
    .split("/")
    .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function resolvePointer(value: unknown, pointer: string): unknown {
  const segments = pointerSegments(pointer);
  if (segments === undefined) return undefined;
  let current = value;
  for (const segment of segments) {
    if (Array.isArray(current)) {
      if (!/^(?:0|[1-9]\d*)$/.test(segment)) return undefined;
      current = current[Number(segment)];
    } else if (current !== null && typeof current === "object") {
      if (!Object.hasOwn(current, segment)) return undefined;
      current = (current as Record<string, unknown>)[segment];
    } else {
      return undefined;
    }
  }
  return current;
}

function unwrapPage(value: unknown, wrap: PaginationDescriptor["wrap"]): unknown {
  let body = value;
  if (wrap.status) body = (body as PageBody | null)?.value;
  if (wrap.headers) body = (body as PageBody | null)?.value;
  return body;
}

function advanceOffset(offset: unknown, count: number): unknown {
  if (typeof offset === "bigint") return offset + BigInt(count);
  if (typeof offset === "string") return String(Number(offset) + count);
  return Number(offset ?? 0) + count;
}

export async function* paginate<Item, Failure>(
  fetchPage: (overrides: PaginationOverrides) => Promise<Result<unknown, Failure>>,
  pagination: PaginationDescriptor,
): AsyncIterable<Result<Item, Failure>> {
  let first = true;
  let cursor: unknown;
  let offset = pagination.kind === "offset" ? pagination.initial : undefined;
  while (true) {
    const overrides: PaginationOverrides = { query: {}, body: {} };
    if (first && pagination.pageSize?.default !== undefined && pagination.pageSize.initial === undefined) {
      overrides[pagination.pageSize.carrier][pagination.pageSize.field] = pagination.pageSize.default;
    }
    if (!first && pagination.kind === "cursor") {
      overrides[pagination.carrier][pagination.field] = cursor;
    }
    if (!first && pagination.kind === "offset") {
      overrides[pagination.carrier][pagination.field] = offset;
    }
    const result = await fetchPage(overrides);
    if (result.isErr()) {
      const failure = result.error;
      if (failure !== undefined) yield err(failure);
      return;
    }
    const body = unwrapPage(result.value, pagination.wrap);
    const items = resolvePointer(body, pagination.items);
    if (items !== null && items !== undefined && !Array.isArray(items)) return;
    const page: unknown[] = Array.isArray(items) ? items : [];
    for (const item of page) yield ok(item as Item);
    if (pagination.kind === "cursor") {
      const next = pagination.next === undefined ? undefined : resolvePointer(body, pagination.next);
      if (next === undefined || next === null || next === "") return;
      cursor = next;
    } else {
      if (page.length === 0) return;
      offset = advanceOffset(offset, page.length);
    }
    first = false;
  }
}
