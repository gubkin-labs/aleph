/**
 * Plan recording. Handlers run once, at build time, inside a recorder. Every operation they call
 * appends a statement to the current block instead of doing anything.
 */
import type { Expr as ExprNode, Op, Stmt } from "@aleph/manifest";

/** Thrown when the SDK is used in a way that can't produce a valid plan. */
export class AlephBuildError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AlephBuildError";
  }
}

export class Recorder {
  #nextVar = 0;
  readonly #blocks: Stmt[][] = [[]];
  #pureDepth = 0;

  /** Allocates a fresh variable id. Ids are unique per plan. */
  newVar(): number {
    return this.#nextVar++;
  }

  push(stmt: Stmt): void {
    const block = this.#blocks.at(-1);
    if (!block) throw new AlephBuildError("internal: no open block");
    block.push(stmt);
  }

  /** Records `op`, binds its result to a new variable, and returns a reference to it. */
  let(op: Op): ExprNode {
    if (this.#pureDepth > 0 && (op.op === "insert" || op.op === "update" || op.op === "delete")) {
      throw new AlephBuildError(
        `can't ${op.op} inside map/filter: their bodies may read but never write. ` +
          "Compute values with map, then write them with $.forEach.",
      );
    }
    const id = this.newVar();
    this.push({ kind: "let", var: id, op });
    return { kind: "var", id };
  }

  /** Runs `fn` with a new open block and returns the statements it recorded. */
  block(fn: () => void, options: { pure?: boolean } = {}): Stmt[] {
    const block: Stmt[] = [];
    this.#blocks.push(block);
    if (options.pure) this.#pureDepth++;
    try {
      fn();
    } finally {
      if (options.pure) this.#pureDepth--;
      this.#blocks.pop();
    }
    return block;
  }

  /** The statements of the top-level block. */
  get body(): Stmt[] {
    const body = this.#blocks[0];
    if (!body) throw new AlephBuildError("internal: no body");
    return body;
  }

  /** The last statement of the current block, if any. */
  last(): Stmt | undefined {
    return this.#blocks.at(-1)?.at(-1);
  }
}

let current: Recorder | undefined;

/** The recorder of the handler being built. Throws outside handlers. */
export function recorder(): Recorder {
  if (!current) {
    throw new AlephBuildError(
      "data operations (get, insert, query, ...) can only be used inside a route handler",
    );
  }
  return current;
}

/** Runs `fn` with `rec` as the current recorder. */
export function recording<T>(rec: Recorder, fn: () => T): T {
  const previous = current;
  current = rec;
  try {
    return fn();
  } finally {
    current = previous;
  }
}
