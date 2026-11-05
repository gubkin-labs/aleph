/**
 * Conversion from SDK values (expressions, rows, lists, plain objects, literals) to manifest
 * expression nodes.
 */
import type { Literal, Expr as Node } from "@aleph/manifest";
import { AlephBuildError } from "./recorder.js";

/**
 * Every SDK value that stands for a plan expression carries its node under this key. It can't
 * collide with a field name, since field names are identifiers.
 */
export const NODE = "~node";

export interface HasNode {
  readonly "~node": Node;
}

export function hasNode(value: unknown): value is HasNode {
  return typeof value === "object" && value !== null && NODE in value;
}

export function literal(value: Literal): Node {
  if (typeof value === "number" && !Number.isSafeInteger(value)) {
    throw new AlephBuildError(
      `${value} isn't a safe integer: Aleph numbers are 64-bit integers (no floats in format 0.1)`,
    );
  }
  return { kind: "lit", value };
}

/** Converts any SDK value to an expression node. */
export function toNode(value: unknown): Node {
  if (hasNode(value)) return value[NODE];
  if (value === null || value === undefined) return literal(null);
  if (typeof value === "string" || typeof value === "boolean" || typeof value === "number") {
    return literal(value);
  }
  if (Array.isArray(value)) return { kind: "list", items: value.map(toNode) };
  if (typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype) {
    const fields: Record<string, Node> = {};
    for (const [key, v] of Object.entries(value)) fields[key] = toNode(v);
    return { kind: "object", fields };
  }
  throw new AlephBuildError(`can't use ${String(value)} (${typeof value}) in a plan`);
}

/** Converts a constant to a literal, rejecting expressions. */
export function toLiteral(value: unknown): Literal {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean" ||
    typeof value === "number"
  ) {
    return value;
  }
  throw new AlephBuildError(`expected a constant, got ${typeof value}`);
}
