/**
 * The scenario format, shared with the Rust runner (`crates/aleph-core/tests/conformance.rs`).
 * Keep both implementations of `substitute`, `subsetMismatch`, and `valueAt` in sync.
 */

export interface Scenario {
  /** Seed for generated ids. */
  seed: number;
  /** Time of the first step, in ms. */
  startMs: number;
  /** Time between steps, in ms. */
  stepMs: number;
  steps: Step[];
}

export interface Step {
  name: string;
  /** A request. Strings may contain `{{var}}` placeholders filled from earlier captures. */
  request: unknown;
  expect: {
    status: number;
    /** A subset of the response body: objects match by the listed keys, arrays item by item. */
    body?: unknown;
    /** Whether the response must (or must not) be an idempotent replay. */
    replayed?: boolean;
  };
  /** Variables to capture from the response, by dot path (`body.items.0.id`). */
  capture?: Record<string, string>;
}

/** Replaces `{{var}}` placeholders in every string of a JSON value. */
export function substitute(value: unknown, vars: Record<string, string>): unknown {
  if (typeof value === "string") {
    return value.replace(/\{\{(\w+)\}\}/g, (_, name: string) => {
      const replacement = vars[name];
      if (replacement === undefined) throw new Error(`unknown scenario variable ${name}`);
      return replacement;
    });
  }
  if (Array.isArray(value)) return value.map((v) => substitute(v, vars));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, substitute(v, vars)]));
  }
  return value;
}

/** Returns a description of the first difference, or `undefined` if `expected` ⊆ `actual`. */
export function subsetMismatch(
  expected: unknown,
  actual: unknown,
  path = "body",
): string | undefined {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) return `${path}: expected an array`;
    if (expected.length !== actual.length) {
      return `${path}: expected ${expected.length} items, got ${actual.length}`;
    }
    for (const [i, item] of expected.entries()) {
      const mismatch = subsetMismatch(item, actual[i], `${path}.${i}`);
      if (mismatch) return mismatch;
    }
    return undefined;
  }
  if (expected !== null && typeof expected === "object") {
    if (actual === null || typeof actual !== "object" || Array.isArray(actual)) {
      return `${path}: expected an object`;
    }
    for (const [key, value] of Object.entries(expected)) {
      const mismatch = subsetMismatch(
        value,
        (actual as Record<string, unknown>)[key],
        `${path}.${key}`,
      );
      if (mismatch) return mismatch;
    }
    return undefined;
  }
  return Object.is(expected, actual)
    ? undefined
    : `${path}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`;
}

/** Reads a dot path (`body.items.0.id`) from a JSON value. */
export function valueAt(value: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((current, key) => {
    if (current === null || typeof current !== "object") return undefined;
    return (current as Record<string, unknown>)[key];
  }, value);
}
