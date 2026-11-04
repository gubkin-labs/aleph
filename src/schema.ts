import type { SchemaNode } from "./manifest.js";

export class SchemaBuilder<Value = unknown> {
  constructor(readonly node: SchemaNode) {}

  max(value: number): SchemaBuilder<Value> {
    if (this.node.type !== "string") {
      throw new TypeError("The max() constraint is only valid for string schemas");
    }

    if (!Number.isInteger(value) || value < 1) {
      throw new RangeError("String max length must be a positive integer");
    }

    return new SchemaBuilder({ ...this.node, maxLength: value });
  }

  min(value: number): SchemaBuilder<Value> {
    if (this.node.type === "string") {
      if (!Number.isInteger(value) || value < 0) {
        throw new RangeError("String min length must be a non-negative integer");
      }

      return new SchemaBuilder({ ...this.node, minLength: value });
    }

    if (this.node.type === "number") {
      return new SchemaBuilder({ ...this.node, min: value });
    }

    throw new TypeError("The min() constraint is only valid for strings and numbers");
  }

  optional(): SchemaBuilder<Value | undefined> {
    return new SchemaBuilder({ ...this.node, optional: true });
  }

  uniqueWithin(scope: string): SchemaBuilder<Value> {
    if (this.node.type !== "string") {
      throw new TypeError("The uniqueWithin() constraint is only valid for strings");
    }

    if (scope.trim().length === 0) {
      throw new RangeError("Uniqueness scope must not be empty");
    }

    return new SchemaBuilder({ ...this.node, uniqueWithin: scope });
  }
}

export type SchemaShape = Readonly<Record<string, SchemaBuilder<unknown>>>;

export type InferSchema<Schema> = Schema extends SchemaBuilder<infer Value>
  ? Value
  : never;

export type InferShape<Shape extends SchemaShape> = {
  [Key in keyof Shape]: InferSchema<Shape[Key]>;
};

export const schema = {
  array: <Value>(items: SchemaBuilder<Value>): SchemaBuilder<Value[]> =>
    new SchemaBuilder({ type: "array", items: items.node }),
  boolean: (): SchemaBuilder<boolean> =>
    new SchemaBuilder({ type: "boolean" }),
  decimal: (params: {
    precision: number;
    scale: number;
  }): SchemaBuilder<string> => {
    if (
      !Number.isInteger(params.precision) ||
      params.precision < 1 ||
      !Number.isInteger(params.scale) ||
      params.scale < 0 ||
      params.scale > params.precision
    ) {
      throw new RangeError("Decimal scale must be between zero and precision");
    }

    return new SchemaBuilder({
      precision: params.precision,
      scale: params.scale,
      type: "decimal",
    });
  },
  email: (): SchemaBuilder<string> =>
    new SchemaBuilder({ format: "email", type: "string" }),
  enum: <const Values extends readonly [string, ...string[]]>(
    values: Values
  ): SchemaBuilder<Values[number]> =>
    new SchemaBuilder({ type: "enum", values: [...values] }),
  number: (): SchemaBuilder<number> => new SchemaBuilder({ type: "number" }),
  object: <const Shape extends SchemaShape>(
    properties: Shape
  ): SchemaBuilder<InferShape<Shape>> => {
    const nodeProperties: Record<string, SchemaNode> = {};

    for (const name of Object.keys(properties)) {
      const property = properties[name];
      if (property === undefined) {
        throw new TypeError(`Schema property '${name}' is undefined`);
      }
      nodeProperties[name] = property.node;
    }

    return new SchemaBuilder({
      properties: nodeProperties,
      type: "object",
    });
  },
  string: (): SchemaBuilder<string> => new SchemaBuilder({ type: "string" }),
  uuid: (): SchemaBuilder<string> =>
    new SchemaBuilder({ format: "uuid", type: "string" }),
};
