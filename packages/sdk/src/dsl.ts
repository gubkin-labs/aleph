/**
 * `$`: control flow, guards, and value helpers. Native `if`, `for`, and operators run at build
 * time, so plans use these instead.
 */

import type { Stmt } from "@aleph/manifest";
import { type ErrorRef, forEach, type List } from "./data.js";
import { Expr, type ExprLike, expr } from "./expr.js";
import { toNode } from "./node.js";
import { AlephBuildError, recorder } from "./recorder.js";

type IfStmt = Extract<Stmt, { kind: "if" }>;

/** Cases of a match: every value of `V`, or some of them plus a `_` default. */
export type MatchCases<V extends string, R> =
  | { [K in V]: ExprLike<R> }
  | ({ [K in V]?: ExprLike<R> } & { _: ExprLike<R> });

class ElseBuilder {
  readonly #stmt: IfStmt;

  constructor(stmt: IfStmt) {
    this.#stmt = stmt;
  }

  /** Statements run when the condition is false. */
  else(fn: () => void): void {
    const rec = recorder();
    if (rec.last() !== this.#stmt) {
      throw new AlephBuildError(".else() must directly follow its .then()");
    }
    this.#stmt.else = rec.block(fn);
  }
}

class IfBuilder {
  readonly #cond: Expr<boolean>;

  constructor(cond: Expr<boolean>) {
    this.#cond = cond;
  }

  /** Statements run when the condition is true. */
  // biome-ignore lint/suspicious/noThenProperty: `$.if(c).then(...)` is the documented API; an IfBuilder is never awaited.
  then(fn: () => void): ElseBuilder {
    const rec = recorder();
    // biome-ignore lint/suspicious/noThenProperty: `then` is a field of the manifest's `if` statement.
    const stmt: IfStmt = { kind: "if", cond: toNode(this.#cond), then: rec.block(fn), else: [] };
    rec.push(stmt);
    return new ElseBuilder(stmt);
  }
}

function fold(op: "and" | "or", conds: ExprLike<boolean>[]): Expr<boolean> {
  const [first, ...rest] = conds;
  if (first === undefined) return expr(op === "and");
  return rest.reduce<Expr<boolean>>((acc, c) => acc[op](c), expr(first));
}

/** Control flow and helpers for handlers. */
export const $ = {
  /** Fails the request with `error` unless `cond` is true. The transaction rolls back. */
  ensure(cond: ExprLike<boolean>, error: ErrorRef): void {
    recorder().push({ kind: "ensure", cond: toNode(cond), error: error.name });
  },

  /** Fails the request with `error`. The transaction rolls back. */
  fail(error: ErrorRef): void {
    recorder().push({ kind: "fail", error: error.name });
  },

  /** Runs `fn` only when `cond` is true. */
  when(cond: ExprLike<boolean>, fn: () => void): void {
    new IfBuilder(expr(cond)).then(fn);
  },

  /** `$.if(cond).then(() => …).else(() => …)`. */
  if(cond: ExprLike<boolean>): IfBuilder {
    return new IfBuilder(expr(cond));
  },

  /** Runs `fn` once per item of a bounded list. The body may write. */
  forEach<T>(list: List<T>, fn: (item: T) => void): void {
    forEach(list, fn);
  },

  /** Chooses a value: `then` if `cond` is true, else `otherwise`. */
  select<A, B>(cond: ExprLike<boolean>, then: ExprLike<A>, otherwise: ExprLike<B>): Expr<A | B> {
    return new Expr({
      kind: "select",
      cond: toNode(cond),
      // biome-ignore lint/suspicious/noThenProperty: `then` is a field of the manifest's `select` expression.
      then: toNode(then),
      else: toNode(otherwise),
    });
  },

  /** Chooses a value by matching a string. Enum matches must be exhaustive or have `_`. */
  match<V extends string, R>(on: Expr<V>, cases: MatchCases<V, R>): Expr<R> {
    const nodes: Record<string, ReturnType<typeof toNode>> = {};
    for (const [key, value] of Object.entries(cases)) nodes[key] = toNode(value);
    return new Expr({ kind: "match", on: toNode(on), cases: nodes });
  },

  /** `true` if every condition is true. */
  all(...conds: ExprLike<boolean>[]): Expr<boolean> {
    return fold("and", conds);
  },

  /** `true` if any condition is true. */
  any(...conds: ExprLike<boolean>[]): Expr<boolean> {
    return fold("or", conds);
  },

  /** The smaller of two ints. `$.min(query.limit, 50)` gives a limit a static bound. */
  min(a: ExprLike<number>, b: ExprLike<number>): Expr<number> {
    return new Expr({ kind: "binary", op: "min", lhs: toNode(a), rhs: toNode(b) });
  },

  /** The larger of two ints. */
  max(a: ExprLike<number>, b: ExprLike<number>): Expr<number> {
    return new Expr({ kind: "binary", op: "max", lhs: toNode(a), rhs: toNode(b) });
  },

  /** String interpolation: `` $.fmt`Hello ${user.name}` ``. */
  fmt(strings: TemplateStringsArray, ...values: unknown[]): Expr<string> {
    const parts = strings.flatMap((s, i) => {
      const literal = s === "" ? [] : [toNode(s)];
      return i < values.length ? [...literal, toNode(values[i])] : literal;
    });
    return new Expr({ kind: "concat", parts });
  },

  /** The signed-in caller's user id, or `null`. */
  get caller(): Expr<string> {
    return new Expr({ kind: "caller" });
  },

  /** The caller's role in an org, or `null` if they aren't a member. */
  callerRole(org: ExprLike<string>): Expr<string | null> {
    return new Expr({ kind: "callerRole", org: toNode(org) });
  },

  /** The current time in milliseconds. Fixed for the whole request. */
  now(): Expr<number> {
    return new Expr({ kind: "now" });
  },
};

export type { List };
