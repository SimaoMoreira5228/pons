export function declaresStatus<T extends { status: number | string }>(entries: T[], actual: number): boolean {
  return findSpecific(entries, actual) !== undefined;
}

function findSpecific<T extends { status: number | string }>(entries: T[], actual: number): T | undefined {
  return entries.find((entry) => entry.status === actual)
    ?? entries.find((entry) => typeof entry.status === "string" && entry.status === `${Math.floor(actual / 100)}xx`);
}

function findCatchAll<T extends { status: number | string }>(entries: T[]): T | undefined {
  return entries.find((entry) => entry.status === "unknown")
    ?? entries.find((entry) => entry.status === "default");
}

export function findError<E extends { status: number | string }>(entries: E[], actual: number): E | undefined {
  return findSpecific(entries, actual) ?? findCatchAll(entries);
}

export function matchResponse<T extends { status: number | string }, E extends { status: number | string }>(
  responses: T[],
  errors: E[],
  actual: number,
): { success: T | undefined; error: E | undefined } {
  const success = findSpecific(responses, actual);
  if (success !== undefined) return { success, error: undefined };
  const error = findError(errors, actual);
  if (error !== undefined) return { success: undefined, error };
  return { success: findCatchAll(responses), error: undefined };
}
