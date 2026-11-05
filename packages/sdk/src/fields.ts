/**
 * Field types: `t.string().max(120)`, `t.int().min(0)`, `t.enum("a", "b")`, ...
 *
 * Every field has a statically known maximum size. That's what lets the core bound row sizes
 * and bytes read, so `t.string()` isn't a field until it has a `.max()`.
 */
import type { DefaultValue, Field, FieldType, Literal } from "@aleph/manifest";
import { toLiteral } from "./node.js";
import { AlephBuildError } from "./recorder.js";

/**
 * A field declaration whose runtime value has type `V`. `O` is whether it's optional (may be
 * `null`), and `D` is whether it has a default (so inserts may omit it).
 */
export class FieldBuilder<V, O extends boolean = false, D extends boolean = false> {
  declare readonly __value?: V;
  declare readonly __optional?: O;
  declare readonly __default?: D;
  readonly spec: Field;

  constructor(spec: Field) {
    this.spec = spec;
  }

  /** Allows the field to be absent (stored as `null`). */
  optional(): FieldBuilder<V, true, D> {
    return new FieldBuilder({ ...this.spec, optional: true });
  }

  /** The value used when the field isn't provided. */
  default(value: V): FieldBuilder<V, O, true> {
    const literal: DefaultValue = { kind: "literal", value: toLiteral(value) };
    return new FieldBuilder({ ...this.spec, default: literal });
  }
}

/** Any field declaration. */
export type AnyField = FieldBuilder<unknown, boolean, boolean>;

/** The runtime value type of a field. */
export type FieldValue<F> =
  F extends FieldBuilder<infer V, infer O, boolean> ? (O extends true ? V | null : V) : never;

/** The names of fields an insert must provide: neither optional nor defaulted. */
export type RequiredFields<F extends Fields> = {
  [K in keyof F]: F[K] extends FieldBuilder<unknown, infer O, infer D>
    ? O extends true
      ? never
      : D extends true
        ? never
        : K
    : never;
}[keyof F];

/** A record of field declarations. */
export type Fields = Record<string, AnyField>;

/** The runtime values of a record of fields. */
export type Values<F extends Fields> = { [K in keyof F]: FieldValue<F[K]> };

const field = <V>(type: FieldType): FieldBuilder<V> => new FieldBuilder<V>({ type });

/** A string field that still needs a maximum length. */
export class StringNeedsMax {
  readonly #min: number;

  constructor(min = 0) {
    this.#min = min;
  }

  /** Minimum length in characters. */
  min(min: number): StringNeedsMax {
    return new StringNeedsMax(min);
  }

  /** Maximum length in characters. Required: it bounds the row size. */
  max(max: number): StringField {
    if (!Number.isInteger(max) || max < this.#min) {
      throw new AlephBuildError(`string max must be an integer ≥ min (${this.#min}), got ${max}`);
    }
    return new StringField({ type: { kind: "string", min: this.#min, max } });
  }
}

export class StringField extends FieldBuilder<string> {
  /** Minimum length in characters. */
  min(min: number): StringField {
    const type = this.spec.type;
    if (type.kind !== "string") throw new AlephBuildError("internal: not a string field");
    return new StringField({ ...this.spec, type: { ...type, min } });
  }
}

export class IntField extends FieldBuilder<number> {
  #withBounds(bounds: { min?: number; max?: number }): IntField {
    const type = this.spec.type;
    if (type.kind !== "int") throw new AlephBuildError("internal: not an int field");
    for (const bound of Object.values(bounds)) {
      if (!Number.isSafeInteger(bound))
        throw new AlephBuildError(`int bounds must be integers, got ${bound}`);
    }
    return new IntField({ ...this.spec, type: { ...type, ...bounds } });
  }

  /** Inclusive minimum. */
  min(min: number): IntField {
    return this.#withBounds({ min });
  }

  /** Inclusive maximum. A max also makes the value usable as a query limit or list bound. */
  max(max: number): IntField {
    return this.#withBounds({ max });
  }
}

export class IdField extends FieldBuilder<string> {
  /** Generates a new time-ordered id when the field isn't provided. */
  generated(): FieldBuilder<string, false, true> {
    return new FieldBuilder({ ...this.spec, default: { kind: "newId" } });
  }
}

export class TimestampField extends FieldBuilder<number> {
  /** Uses the current time when the field isn't provided. */
  defaultNow(): FieldBuilder<number, false, true> {
    return new FieldBuilder({ ...this.spec, default: { kind: "now" } });
  }
}

/** Field type constructors. */
export const t = {
  /** A string. Call `.max(n)` to make it a field. */
  string: (): StringNeedsMax => new StringNeedsMax(),
  /** A 64-bit integer. */
  int: (): IntField => new IntField({ type: { kind: "int" } }),
  /** A boolean. */
  bool: (): FieldBuilder<boolean> => field({ kind: "bool" }),
  /** One of a fixed set of strings. */
  enum: <const V extends string>(...values: [V, ...V[]]): FieldBuilder<V> =>
    field({ kind: "enum", values }),
  /** A time-ordered id (ULID). Call `.generated()` to create ids automatically. */
  id: (): IdField => new IdField({ type: { kind: "id" } }),
  /** Milliseconds since the Unix epoch. */
  timestamp: (): TimestampField => new TimestampField({ type: { kind: "timestamp" } }),
  /** A list of at most `max` items. */
  array: <V>(items: FieldBuilder<V>, max: number): FieldBuilder<V[]> => {
    if (items.spec.optional || items.spec.default) {
      throw new AlephBuildError("array items can't be optional or have defaults");
    }
    return field({ kind: "array", items: items.spec.type, max });
  },
};

/** Builds a manifest field record from field declarations. */
export function fieldSpecs(fields: Fields): Record<string, Field> {
  const out: Record<string, Field> = {};
  for (const [name, f] of Object.entries(fields)) out[name] = f.spec;
  return out;
}

export type { Literal };
