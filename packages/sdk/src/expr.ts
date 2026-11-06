/**
 * Expressions. An `Expr<T>` stands for a value of type `T` that only exists when the plan runs,
 * so it has methods instead of operators: `a.plus(b).gte(c)`, not `a + b >= c`.
 */
import type { BinaryOp, Expr as Node, Rounding, TimePart, TimeUnit } from "@aleph/manifest";
import { List } from "./data.js";
import type { Decimal } from "./fields.js";
import { toLiteral, toNode } from "./node.js";
import { AlephBuildError } from "./recorder.js";

/** The result of a string operation: `null` in, `null` out. */
type StrResult<T> = null extends T ? string | null : string;

/** A number a plan computes with: an integer or an exact decimal. */
export type Numeric = number | Decimal;

/** A value accepted where an `Expr<T>` is expected: the expression or a constant. */
export type ExprLike<T> = Expr<T> | T;

/** An IANA time zone: a name like `"Europe/Berlin"`, or an expression (`null` means UTC). */
export type TimeZoneLike = ExprLike<string | null>;

const misuse = (what: string) =>
  new AlephBuildError(
    `an Aleph expression was used as a plain JavaScript value (${what}). Expressions only exist ` +
      "when the plan runs: use methods like .eq(), .plus() and $.fmt`...` instead of operators, " +
      "and $.when / $.select / $.ensure instead of `if`.",
  );

function tzNode(tz: TimeZoneLike | undefined): { tz?: Node } {
  return tz === undefined ? {} : { tz: toNode(tz) };
}

export class Expr<T> {
  declare readonly __type?: T;
  readonly "~node": Node;

  constructor(node: Node) {
    this["~node"] = node;
  }

  [Symbol.toPrimitive](hint: string): never {
    throw misuse(`converted to ${hint}`);
  }

  toString(): never {
    throw misuse("converted to string");
  }

  valueOf(): never {
    throw misuse("converted to a primitive");
  }

  #binary<R>(op: BinaryOp, other: unknown): Expr<R> {
    return new Expr<R>({ kind: "binary", op, lhs: this["~node"], rhs: toNode(other) });
  }

  eq(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("eq", other);
  }
  neq(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("neq", other);
  }
  lt(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("lt", other);
  }
  lte(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("lte", other);
  }
  gt(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("gt", other);
  }
  gte(other: ExprLike<T>): Expr<boolean> {
    return this.#binary("gte", other);
  }

  /** `true` if the value equals one of `values`. */
  in(values: readonly T[]): Expr<boolean> {
    return new Expr({ kind: "in", value: this["~node"], list: values.map(toLiteral) });
  }

  /** `true` if the value isn't one of `values`. */
  notIn(values: readonly T[]): Expr<boolean> {
    return this.in(values).not();
  }

  isNull(): Expr<boolean> {
    return new Expr({ kind: "isNull", of: this["~node"] });
  }

  isSet(): Expr<boolean> {
    return this.isNull().not();
  }

  /** The value, or `fallback` when it's `null`. */
  ifNull<F extends NonNullable<T>>(fallback: ExprLike<F>): Expr<NonNullable<T>> {
    return new Expr({
      kind: "select",
      cond: this.isNull()["~node"],
      // biome-ignore lint/suspicious/noThenProperty: `then` is a field of the manifest's `select` expression.
      then: toNode(fallback),
      else: this["~node"],
    });
  }

  and(this: Expr<boolean>, other: ExprLike<boolean>): Expr<boolean> {
    return this.#binary("and", other);
  }
  or(this: Expr<boolean>, other: ExprLike<boolean>): Expr<boolean> {
    return this.#binary("or", other);
  }
  not(this: Expr<boolean>): Expr<boolean> {
    return new Expr({ kind: "not", arg: this["~node"] });
  }

  // ─── Numbers: ints stay ints; a decimal on either side makes the result a decimal ──────────

  plus(this: Expr<number>, other: ExprLike<number>): Expr<number>;
  plus(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Decimal>;
  plus(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Numeric> {
    return this.#binary("add", other);
  }
  minus(this: Expr<number>, other: ExprLike<number>): Expr<number>;
  minus(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Decimal>;
  minus(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Numeric> {
    return this.#binary("sub", other);
  }
  /** Multiplication. Decimal scales add up (`2.50 × 1.08 = 2.7000`), up to 18 digits. */
  times(this: Expr<number>, other: ExprLike<number>): Expr<number>;
  times(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Decimal>;
  times(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Numeric> {
    return this.#binary("mul", other);
  }
  /**
   * Division. Ints truncate toward zero; decimals keep the larger scale, rounding half-even
   * (`.round(n)` first for more digits).
   */
  div(this: Expr<number>, other: ExprLike<number>): Expr<number>;
  div(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Decimal>;
  div(this: Expr<Numeric>, other: ExprLike<Numeric>): Expr<Numeric> {
    return this.#binary("div", other);
  }
  /** Integer division rounding down (toward negative infinity). */
  divFloor(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("divFloor", other);
  }
  /** Integer division rounding up: `items.divCeil(pageSize)` pages. */
  divCeil(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("divCeil", other);
  }
  /** The remainder, always between 0 and `|other|` (so `-1 mod 7` is 6). */
  mod(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("mod", other);
  }
  /** Integer power; the exponent must be 0-63. */
  pow(this: Expr<number>, exponent: ExprLike<number>): Expr<number> {
    return this.#binary("pow", exponent);
  }
  negate<N extends Numeric>(this: Expr<N>): Expr<N> {
    return new Expr({ kind: "neg", arg: this["~node"] });
  }

  /** Absolute value. */
  abs<N extends Numeric>(this: Expr<N>): Expr<N> {
    return new Expr({ kind: "abs", arg: this["~node"] });
  }

  /** The number as a decimal with `scale` digits after the point. */
  round(this: Expr<Numeric | null>, scale: number, mode: Rounding = "halfEven"): Expr<Decimal> {
    return new Expr({
      kind: "round",
      of: this["~node"],
      scale,
      ...(mode === "halfEven" ? {} : { mode }),
    });
  }

  /** The number as an int (rounding half-even unless told otherwise). */
  toInt(this: Expr<Numeric | null>, mode: Rounding = "halfEven"): Expr<number> {
    return new Expr({ kind: "toInt", of: this["~node"], ...(mode === "halfEven" ? {} : { mode }) });
  }

  /** Formats a number: `format({ decimals: 2, grouping: true })` gives `"1,234.50"`. */
  format(
    this: Expr<Numeric | null>,
    options: { decimals?: number; grouping?: boolean } = {},
  ): Expr<string> {
    return new Expr({
      kind: "formatNumber",
      of: this["~node"],
      ...(options.decimals === undefined ? {} : { decimals: options.decimals }),
      ...(options.grouping ? { grouping: true } : {}),
    });
  }

  // ─── Strings ───────────────────────────────────────────────────────────────────────────────

  /** Lowercase (`null` stays `null`). */
  lower<S extends string | null>(this: Expr<S>): Expr<StrResult<S>> {
    return new Expr({ kind: "str", op: "lower", of: this["~node"] });
  }

  /** Uppercase. */
  upper<S extends string | null>(this: Expr<S>): Expr<StrResult<S>> {
    return new Expr({ kind: "str", op: "upper", of: this["~node"] });
  }

  /** Without leading and trailing whitespace. */
  trim<S extends string | null>(this: Expr<S>): Expr<StrResult<S>> {
    return new Expr({ kind: "str", op: "trim", of: this["~node"] });
  }

  /** The first `max` characters. */
  truncate<S extends string | null>(this: Expr<S>, max: number): Expr<StrResult<S>> {
    return new Expr({ kind: "truncate", of: this["~node"], max });
  }

  /** Characters from `start` up to `end`, like JavaScript's `slice` (negatives count from the end). */
  slice<S extends string | null>(
    this: Expr<S>,
    start: ExprLike<number>,
    end?: ExprLike<number>,
  ): Expr<StrResult<S>> {
    return new Expr({
      kind: "slice",
      of: this["~node"],
      start: toNode(start),
      ...(end === undefined ? {} : { end: toNode(end) }),
    });
  }

  /** Splits on `separator` into at most `max` parts (the last keeps the rest). */
  split(this: Expr<string | null>, separator: ExprLike<string>, max: number): List<Expr<string>> {
    return new List<Expr<string>>(
      { kind: "split", of: this["~node"], sep: toNode(separator), max },
      { kind: "scalar" },
    );
  }

  /** Every occurrence of `find` replaced with `replacement`. */
  replace<S extends string | null>(
    this: Expr<S>,
    find: ExprLike<string>,
    replacement: ExprLike<string>,
  ): Expr<StrResult<S>> {
    return new Expr({
      kind: "replace",
      of: this["~node"],
      find: toNode(find),
      with: toNode(replacement),
    });
  }

  /** Pads to `length` characters at the start: `n.padStart(4, "0")` gives `"0042"`. */
  padStart(this: Expr<unknown>, length: number, fill = " "): Expr<string> {
    return new Expr({ kind: "pad", of: this["~node"], length, fill });
  }

  /** Pads to `length` characters at the end. */
  padEnd(this: Expr<unknown>, length: number, fill = " "): Expr<string> {
    return new Expr({ kind: "pad", of: this["~node"], length, fill, end: true });
  }

  /** `true` if the string matches a regular expression (matching takes linear time). */
  matches(this: Expr<string | null>, pattern: RegExp | string): Expr<boolean> {
    const source = typeof pattern === "string" ? pattern : pattern.source;
    if (pattern instanceof RegExp && pattern.flags.replace("u", "") !== "") {
      throw new AlephBuildError(
        `regex flags aren't supported (got /${source}/${pattern.flags}); use inline flags like (?i)`,
      );
    }
    return new Expr({ kind: "matches", of: this["~node"], pattern: source });
  }

  /** `false` if either side is `null`. */
  startsWith(this: Expr<string | null>, other: ExprLike<string | null>): Expr<boolean> {
    return new Expr({ kind: "strTest", op: "startsWith", of: this["~node"], arg: toNode(other) });
  }

  /** `false` if either side is `null`. */
  endsWith(this: Expr<string | null>, other: ExprLike<string | null>): Expr<boolean> {
    return new Expr({ kind: "strTest", op: "endsWith", of: this["~node"], arg: toNode(other) });
  }

  /** `false` if either side is `null`. */
  contains(this: Expr<string | null>, other: ExprLike<string | null>): Expr<boolean> {
    return new Expr({ kind: "strTest", op: "contains", of: this["~node"], arg: toNode(other) });
  }

  /** Length of a string, in characters. */
  length(this: Expr<string>): Expr<number> {
    return new Expr({ kind: "length", of: this["~node"] });
  }

  // ─── Time (timestamps are milliseconds; time zones default to UTC) ─────────────────────────

  /** The start of the day (week, month, ...) containing this timestamp, in a time zone. */
  startOf(this: Expr<number | null>, unit: TimeUnit, tz?: TimeZoneLike): Expr<number> {
    return new Expr({ kind: "startOf", of: this["~node"], unit, ...tzNode(tz) });
  }

  /**
   * This timestamp moved by `amount` units. Days and larger follow the calendar in the time
   * zone, so adding a day keeps the wall-clock time across DST changes.
   */
  add(
    this: Expr<number | null>,
    amount: ExprLike<number>,
    unit: TimeUnit,
    tz?: TimeZoneLike,
  ): Expr<number> {
    return new Expr({
      kind: "addTime",
      of: this["~node"],
      amount: toNode(amount),
      unit,
      ...tzNode(tz),
    });
  }

  /** One part of this timestamp, like its `weekday` (1 = Monday) or `hour`, in a time zone. */
  part(this: Expr<number | null>, part: TimePart, tz?: TimeZoneLike): Expr<number> {
    return new Expr({ kind: "timePart", of: this["~node"], part, ...tzNode(tz) });
  }

  /** Formats this timestamp: ISO 8601 by default, or a `strftime` pattern like `"%Y-%m-%d"`. */
  formatTime(this: Expr<number | null>, pattern?: string, tz?: TimeZoneLike): Expr<string> {
    return new Expr({
      kind: "formatTime",
      of: this["~node"],
      ...(pattern === undefined ? {} : { pattern }),
      ...tzNode(tz),
    });
  }
}

/** Wraps any SDK value as an expression. */
export function expr<T>(value: ExprLike<T>): Expr<T> {
  return value instanceof Expr ? value : new Expr<T>(toNode(value));
}
