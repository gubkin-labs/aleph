/**
 * Expressions. An `Expr<T>` stands for a value of type `T` that only exists when the plan runs,
 * so it has methods instead of operators: `a.plus(b).gte(c)`, not `a + b >= c`.
 */
import type { BinaryOp, Expr as Node } from "@aleph/manifest";
import { toLiteral, toNode } from "./node.js";
import { AlephBuildError } from "./recorder.js";

/** The result of a string operation: `null` in, `null` out. */
type StrResult<T> = null extends T ? string | null : string;

/** A value accepted where an `Expr<T>` is expected: the expression or a constant. */
export type ExprLike<T> = Expr<T> | T;

const misuse = (what: string) =>
  new AlephBuildError(
    `an Aleph expression was used as a plain JavaScript value (${what}). Expressions only exist ` +
      "when the plan runs: use methods like .eq(), .plus() and $.fmt`...` instead of operators, " +
      "and $.when / $.select / $.ensure instead of `if`.",
  );

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

  and(this: Expr<boolean>, other: ExprLike<boolean>): Expr<boolean> {
    return this.#binary("and", other);
  }
  or(this: Expr<boolean>, other: ExprLike<boolean>): Expr<boolean> {
    return this.#binary("or", other);
  }
  not(this: Expr<boolean>): Expr<boolean> {
    return new Expr({ kind: "not", arg: this["~node"] });
  }

  plus(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("add", other);
  }
  minus(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("sub", other);
  }
  times(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("mul", other);
  }
  div(this: Expr<number>, other: ExprLike<number>): Expr<number> {
    return this.#binary("div", other);
  }
  negate(this: Expr<number>): Expr<number> {
    return new Expr({ kind: "neg", arg: this["~node"] });
  }

  /** Absolute value. */
  abs(this: Expr<number>): Expr<number> {
    return new Expr({ kind: "abs", arg: this["~node"] });
  }

  /** Lowercase (`null` stays `null`). */
  lower<T extends string | null>(this: Expr<T>): Expr<StrResult<T>> {
    return new Expr({ kind: "str", op: "lower", of: this["~node"] });
  }

  /** Uppercase. */
  upper<T extends string | null>(this: Expr<T>): Expr<StrResult<T>> {
    return new Expr({ kind: "str", op: "upper", of: this["~node"] });
  }

  /** Without leading and trailing whitespace. */
  trim<T extends string | null>(this: Expr<T>): Expr<StrResult<T>> {
    return new Expr({ kind: "str", op: "trim", of: this["~node"] });
  }

  /** The first `max` characters. */
  truncate<T extends string | null>(this: Expr<T>, max: number): Expr<StrResult<T>> {
    return new Expr({ kind: "truncate", of: this["~node"], max });
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
}

/** Wraps any SDK value as an expression. */
export function expr<T>(value: ExprLike<T>): Expr<T> {
  return value instanceof Expr ? value : new Expr<T>(toNode(value));
}
