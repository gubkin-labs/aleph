/**
 * Plan recording. Handlers run once, at build time, inside a recorder. Every operation they call
 * appends a statement to the current block instead of doing anything.
 */
import { relative } from "node:path";
import { fileURLToPath } from "node:url";
import type { Expr as ExprNode, Op, Stmt } from "@aleph-lang/manifest";

/** Thrown when the SDK is used in a way that can't produce a valid plan. */
export class AlephBuildError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AlephBuildError";
  }
}

/** Where an SDK call came from: the first stack frame outside the SDK, relative to the cwd. */
export function callSite(): string | undefined {
  if (typeof process === "undefined") return undefined;
  const sdkRoot = fileURLToPath(new URL("..", import.meta.url));
  for (const line of (new Error().stack ?? "").split("\n").slice(1)) {
    const match = /\(?((?:file:\/\/)?[^\s()]+):(\d+):(\d+)\)?$/.exec(line.trim());
    if (!match?.[1]) continue;
    const file = match[1].startsWith("file://") ? fileURLToPath(match[1]) : match[1];
    if (file.startsWith(sdkRoot) || file.startsWith("node:") || file.includes("/node_modules/")) {
      continue;
    }
    return `${relative(process.cwd(), file)}:${match[2]}:${match[3]}`;
  }
  return undefined;
}

interface OpenBlock {
  stmts: Stmt[];
  path: string;
}

/** Options for {@link Recorder.block}. */
export interface BlockOptions {
  /** A map/filter body: may read, never write. */
  pure?: boolean;
  /** The block's path relative to the statement that will own it, for example `.then`. */
  suffix: string;
  /** The owning statement was already recorded (it's the last one), as for `.else`. */
  ownerRecorded?: boolean;
}

export class Recorder {
  #nextVar = 0;
  readonly #blocks: OpenBlock[];
  #pureDepth = 0;
  /** Manifest path → source location, for diagnostics. */
  readonly sourceMap: Record<string, string> = {};

  /** @param basePath The manifest path of the top-level statements, like `routes[0].plan.body`. */
  constructor(basePath = "") {
    this.#blocks = [{ stmts: [], path: basePath }];
  }

  /** Allocates a fresh variable id. Ids are unique per plan. */
  newVar(): number {
    return this.#nextVar++;
  }

  #top(): OpenBlock {
    const block = this.#blocks.at(-1);
    if (!block) throw new AlephBuildError("internal: no open block");
    return block;
  }

  push(stmt: Stmt): void {
    const block = this.#top();
    const path = `${block.path}[${block.stmts.length}]`;
    block.stmts.push(stmt);
    const site = this.#blocks[0]?.path ? callSite() : undefined;
    if (site) this.sourceMap[path] = site;
  }

  /** Records `op`, binds its result to a new variable, and returns a reference to it. */
  let(op: Op): ExprNode {
    const writes = ["insert", "upsert", "insertIfAbsent", "update", "delete", "deleteBlob"];
    const effects = ["emit", "enqueue", "publish", "send"];
    if (this.#pureDepth > 0 && (writes.includes(op.op) || effects.includes(op.op))) {
      throw new AlephBuildError(
        `can't ${op.op} inside map/filter/loop: their bodies may read but never write or ` +
          "release effects. Compute values first, then act on them with $.forEach.",
      );
    }
    const id = this.newVar();
    this.push({ kind: "let", var: id, op });
    return { kind: "var", id };
  }

  /** Runs `fn` with a new open block and returns the statements it recorded. */
  block(fn: () => void, options: BlockOptions): Stmt[] {
    const parent = this.#top();
    const owner = options.ownerRecorded ? parent.stmts.length - 1 : parent.stmts.length;
    const block: OpenBlock = { stmts: [], path: `${parent.path}[${owner}]${options.suffix}` };
    this.#blocks.push(block);
    if (options.pure) this.#pureDepth++;
    try {
      fn();
    } finally {
      if (options.pure) this.#pureDepth--;
      this.#blocks.pop();
    }
    return block.stmts;
  }

  /** The statements of the top-level block. */
  get body(): Stmt[] {
    const body = this.#blocks[0];
    if (!body) throw new AlephBuildError("internal: no body");
    return body.stmts;
  }

  /** The last statement of the current block, if any. */
  last(): Stmt | undefined {
    return this.#top().stmts.at(-1);
  }
}

let current: Recorder | undefined;

/** The recorder of the handler being built. Throws outside handlers. */
export function recorder(): Recorder {
  if (!current) {
    throw new AlephBuildError(
      "data operations (get, insert, query, emit, ...) can only be used inside a route or handler",
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
