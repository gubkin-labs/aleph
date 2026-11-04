import type { ExpressionNode, JsonValue } from "./manifest.js";

export type ExpressionInput =
  | JsonValue
  | ExpressionBuilder
  | { readonly [key: string]: ExpressionInput }
  | readonly ExpressionInput[];

const isPlainRecord = (
  value: unknown
): value is Readonly<Record<string, unknown>> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
};

const isJsonValue = (value: unknown, depth = 0): value is JsonValue => {
  if (depth > 64) {
    return false;
  }

  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean"
  ) {
    return true;
  }

  if (typeof value === "number") {
    return Number.isFinite(value);
  }

  if (Array.isArray(value)) {
    return value.every((item: unknown) => isJsonValue(item, depth + 1));
  }

  if (!isPlainRecord(value)) {
    return false;
  }

  for (const key of Object.keys(value)) {
    const child = value[key];
    if (child === undefined || !isJsonValue(child, depth + 1)) {
      return false;
    }
  }

  return true;
};

export class ExpressionBuilder {
  constructor(readonly node: ExpressionNode) {}

  add(value: ExpressionInput): ExpressionBuilder {
    return this.combine("add", value);
  }

  eq(value: ExpressionInput): ExpressionBuilder {
    return this.combine("eq", value);
  }

  multiply(value: ExpressionInput): ExpressionBuilder {
    return this.combine("multiply", value);
  }

  private combine(
    operator: "add" | "multiply" | "eq",
    value: ExpressionInput
  ): ExpressionBuilder {
    return new ExpressionBuilder({
      kind: "binary",
      left: this.node,
      operator,
      right: toExpressionNode(value),
    });
  }
}

export const arg = (path: string): ExpressionBuilder => {
  const segments = path.split(".").filter((segment) => segment.length > 0);
  if (segments.length === 0) {
    throw new RangeError("Argument path must not be empty");
  }

  return new ExpressionBuilder({ kind: "argument", path: segments });
};

export const ref = (name: string): ExpressionBuilder =>
  new ExpressionBuilder({ kind: "binding", name });

export const result = ref;

export const toExpressionNode = (value: unknown): ExpressionNode => {
  if (value instanceof ExpressionBuilder) {
    return value.node;
  }

  if (isJsonValue(value)) {
    return { kind: "literal", value };
  }

  if (Array.isArray(value)) {
    return {
      items: value.map((item: unknown) => toExpressionNode(item)),
      kind: "array",
    };
  }

  if (isPlainRecord(value)) {
    const properties: Record<string, ExpressionNode> = {};

    for (const key of Object.keys(value)) {
      const child = value[key];
      if (child === undefined) {
        throw new TypeError(`Expression property '${key}' is undefined`);
      }
      properties[key] = toExpressionNode(child);
    }

    return { kind: "object", properties };
  }

  throw new TypeError("Expression contains a value that cannot be serialized");
};
