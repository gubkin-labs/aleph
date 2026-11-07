/**
 * Asynchronous work: events, queues, channels, integrations, and handlers.
 *
 * Effects (`emit`, `enqueue`, `publish`, `send`) are buffered while a plan runs and released
 * only after its transaction commits, so a failed or retried attempt never releases anything.
 */
import type {
  ChannelDecl,
  CronScan,
  ErrorDecl,
  EventDecl,
  Handler,
  Integration,
  IntegrationOp,
  Expr as Node,
  QueueDecl,
  Trigger,
} from "@aleph-lang/manifest";
import {
  type Accessor,
  type Entity,
  ErrorRef,
  type InsertValues,
  List,
  materialize,
  type Row,
  shapeOfFields,
} from "./data.js";
import { $ } from "./dsl.js";
import { Expr, type ExprLike } from "./expr.js";
import { type FieldBuilder, type Fields, type FieldValue, fieldSpecs } from "./fields.js";
import { toLiteral, toNode } from "./node.js";
import { AlephBuildError, callSite, Recorder, recorder, recording } from "./recorder.js";
import { type InputValues, inputs } from "./route.js";
import { type Duration, durationMs } from "./units.js";

function nodes(values: Record<string, unknown>): Record<string, Node> {
  return Object.fromEntries(Object.entries(values).map(([k, v]) => [k, toNode(v)]));
}

// ─── Events ────────────────────────────────────────────────────────────────────────────────────

/** A declared event. */
export class Event<P extends Fields> {
  readonly name: string;
  readonly payload: P;

  /** @internal */
  constructor(name: string, payload: P) {
    this.name = name;
    this.payload = payload;
  }

  /** @internal */
  spec(): EventDecl {
    return { payload: fieldSpecs(this.payload) };
  }

  /** Emits the event to its handlers, after commit. */
  emit(payload: InsertValues<P>): void {
    recorder().let({ op: "emit", event: this.name, payload: nodes(payload) });
  }
}

// ─── Queues ────────────────────────────────────────────────────────────────────────────────────

/** How to declare a queue. */
export interface QueueOptions<P extends Fields> {
  payload: P;
  /** Retries after the first failure (default 3, at most 10). */
  retries?: number;
  /** Delay before the first retry; doubles each time (default `"1s"`). */
  backoff?: Duration;
  /** The longest delay between retries (default `"1m"`). */
  maxBackoff?: Duration;
}

/** A declared queue. */
export class Queue<P extends Fields> {
  readonly name: string;
  readonly payload: P;
  readonly #options: QueueOptions<P>;

  /** @internal */
  constructor(name: string, options: QueueOptions<P>) {
    this.name = name;
    this.payload = options.payload;
    this.#options = options;
  }

  /** @internal */
  spec(): QueueDecl {
    return {
      payload: fieldSpecs(this.payload),
      retries: this.#options.retries ?? 3,
      backoffMs: durationMs(this.#options.backoff ?? "1s"),
      maxBackoffMs: durationMs(this.#options.maxBackoff ?? "1m"),
    };
  }

  /**
   * Puts a job on the queue, after commit, optionally delayed (at most 30 days). With a `key`,
   * a pending job with the same key is replaced, and {@link Queue.cancel} can remove it.
   */
  enqueue(
    payload: InsertValues<P>,
    options: { delay?: Duration | Expr<number>; key?: ExprLike<string> } = {},
  ): void {
    const delay = options.delay;
    const delayMs =
      delay === undefined ? undefined : delay instanceof Expr ? delay : durationMs(delay);
    recorder().let({
      op: "enqueue",
      queue: this.name,
      payload: nodes(payload),
      ...(delayMs === undefined ? {} : { delayMs: toNode(delayMs) }),
      ...(options.key === undefined ? {} : { key: toNode(options.key) }),
    });
  }

  /** Cancels the pending job with this key, if any, after commit. */
  cancel(key: ExprLike<string>): void {
    recorder().let({ op: "cancel", queue: this.name, key: toNode(key) });
  }
}

// ─── Channels ──────────────────────────────────────────────────────────────────────────────────

/** A realtime channel. */
export class Channel<K extends Fields, M extends Fields> {
  readonly name: string;
  readonly #key: K;
  readonly #message: M;

  /** @internal */
  constructor(name: string, def: { key: K; message: M }) {
    this.name = name;
    this.#key = def.key;
    this.#message = def.message;
  }

  /** @internal */
  spec(): ChannelDecl {
    return { key: fieldSpecs(this.#key), message: fieldSpecs(this.#message) };
  }

  /** Publishes a message to the stream named by `key`, after commit. */
  publish(key: { [P in keyof K]: ExprLike<FieldValue<K[P]>> }, message: InsertValues<M>): void {
    recorder().let({ op: "publish", channel: this.name, key: nodes(key), message: nodes(message) });
  }
}

// ─── Integrations ──────────────────────────────────────────────────────────────────────────────

/** An integration operation declaration. */
export type OperationDef =
  | { mode: "call"; input: Fields; output: Fields }
  | { mode: "effect"; input: Fields };

type CallOps<O> = { [K in keyof O]: O[K] extends { mode: "call" } ? K : never }[keyof O] & string;
type EffectOps<O> = { [K in keyof O]: O[K] extends { mode: "effect" } ? K : never }[keyof O] &
  string;
type InputOf<D> = D extends { input: infer F extends Fields } ? F : never;
type OutputOf<D> = D extends { output: infer F extends Fields } ? F : never;

/** How to declare an integration. */
export interface IntegrationDef<O extends Record<string, OperationDef>> {
  /** The provider that implements it, for example `"http"`, `"email"`, `"stripe"`. */
  provider: string;
  /** Provider configuration (base URL, secret names, ...), interpreted by the platform. */
  config?: Record<string, string>;
  operations: O;
}

/** A declared integration: the only way plans reach the outside world. */
export class IntegrationHandle<O extends Record<string, OperationDef>> {
  readonly name: string;
  readonly #def: IntegrationDef<O>;

  /** @internal */
  constructor(name: string, def: IntegrationDef<O>) {
    this.name = name;
    this.#def = def;
  }

  /** @internal */
  spec(): Integration {
    const operations: Record<string, IntegrationOp> = {};
    for (const [name, op] of Object.entries<OperationDef>(this.#def.operations)) {
      operations[name] = {
        mode: op.mode,
        input: fieldSpecs(op.input),
        ...(op.mode === "call" ? { output: fieldSpecs(op.output) } : {}),
      };
    }
    return { provider: this.#def.provider, config: { ...this.#def.config }, operations };
  }

  /**
   * Calls an operation and returns its validated output. Calls run during the plan and are
   * journaled: a retried attempt reuses the result instead of calling again.
   */
  call<K extends CallOps<O>>(
    operation: K,
    input: InsertValues<InputOf<O[K]>>,
  ): { readonly [F in keyof OutputOf<O[K]>]: Accessor<FieldValue<OutputOf<O[K]>[F]>> } {
    const node = recorder().let({
      op: "call",
      integration: this.name,
      operation,
      input: nodes(input),
    });
    const def = this.#def.operations[operation];
    const output = def && def.mode === "call" ? def.output : {};
    return materialize({ kind: "object", fields: shapeOfFields(output) }, node) as never;
  }

  /** Sends an effect operation, after commit. */
  send<K extends EffectOps<O>>(operation: K, input: InsertValues<InputOf<O[K]>>): void {
    recorder().let({ op: "send", integration: this.name, operation, input: nodes(input) });
  }
}

// ─── Handlers ──────────────────────────────────────────────────────────────────────────────────

/** A handler's context: its payload and declared errors. */
export interface HandlerCtx<P, E extends string> {
  payload: P;
  errors: { readonly [K in E]: ErrorRef };
}

/** The payload of an entity change trigger. */
export interface ChangePayload<F extends Fields> {
  /** The row before the change (check `.exists()`: `null` on insert). */
  before: Row<F>;
  /** The row after the change (check `.exists()`: `null` on delete). */
  after: Row<F>;
}

/** The payload of a cron run. */
export interface CronPayload {
  /** The tick the run belongs to, in ms. */
  firedAt: Expr<number>;
}

/** Handler options. */
export interface HandlerOptions<E extends string> {
  /** Errors the handler may fail with. A failure is retried with backoff. */
  errors?: Record<E, ErrorDecl>;
  /**
   * Allows the handler to be part of a trigger cycle: it writes, emits, or enqueues what
   * eventually triggers it again, and a guard stops the chain. It then runs at most this many
   * handler hops after the request or schedule that started the chain (1-16). Without it, a
   * cycle is a `trigger_cycle` validation error.
   */
  maxDepth?: number;
}

/** @internal Registers handlers with an app. */
export interface HandlerSink {
  next(): number;
  add(handler: Handler, sourceMap: Record<string, string>): void;
}

function payloadInputs<F extends Fields>(fields: F): InputValues<F> {
  return inputs("payload", fields);
}

function rowPayload<F extends Fields>(entity: Entity<F, string, string>, name: string): Row<F> {
  return materialize(
    { kind: "row", entity: entity as never },
    { kind: "input", source: "payload", name },
  ) as Row<F>;
}

/** @internal Records a handler's plan. */
export function recordHandler<P, E extends string>(
  sink: HandlerSink,
  id: string,
  trigger: Trigger,
  payload: P,
  fn: ($dsl: typeof $, ctx: HandlerCtx<P, E>) => unknown,
  options: HandlerOptions<E> = {},
): void {
  const declared = options.errors ?? ({} as Record<E, ErrorDecl>);
  const errors = Object.fromEntries(Object.keys(declared).map((n) => [n, new ErrorRef(n)])) as {
    [K in E]: ErrorRef;
  };
  const index = sink.next();
  const rec = new Recorder(`handlers[${index}].plan.body`);
  const site = callSite();
  if (site) rec.sourceMap[`handlers[${index}]`] = site;
  const output = recording(rec, () => toNode(fn($, { payload, errors })));
  sink.add(
    {
      id,
      trigger,
      errors: { ...declared },
      ...(options.maxDepth === undefined ? {} : { maxDepth: options.maxDepth }),
      plan: { body: rec.body, output },
    },
    rec.sourceMap,
  );
}

/** Builds handler triggers and their typed payloads. */
export const triggers = {
  event<P extends Fields>(event: Event<P>): [Trigger, InputValues<P>] {
    return [{ kind: "event", event: event.name }, payloadInputs(event.payload)];
  },
  queue<P extends Fields>(queue: Queue<P>): [Trigger, InputValues<P>] {
    return [{ kind: "queue", queue: queue.name }, payloadInputs(queue.payload)];
  },
  deadLetter<P extends Fields>(
    queue: Queue<P>,
  ): [Trigger, InputValues<P> & { error: Expr<string>; attempts: Expr<number> }] {
    const payload = {
      ...payloadInputs(queue.payload),
      error: new Expr<string>({ kind: "input", source: "payload", name: "error" }),
      attempts: new Expr<number>({ kind: "input", source: "payload", name: "attempts" }),
    };
    return [{ kind: "deadLetter", queue: queue.name }, payload];
  },
  change<F extends Fields>(
    entity: Entity<F, string, string>,
    change: "inserted" | "updated" | "deleted",
  ): [Trigger, ChangePayload<F>] {
    const payload = { before: rowPayload(entity, "before"), after: rowPayload(entity, "after") };
    return [{ kind: "entity", entity: entity.name, change }, payload];
  },
  schedule<F extends Fields>(
    cron: string,
    tz: string | undefined,
    scan: ScanOptions<F> | undefined,
  ): [Trigger, CronPayload & { batch: List<Row<F>> }] {
    const [trigger, payload] = triggers.cron<F>(60_000, scan ? { scan } : {});
    if (trigger.kind !== "cron") throw new AlephBuildError("internal: not a cron trigger");
    const scheduled: Trigger = {
      ...trigger,
      everyMs: 0,
      offsetMs: 0,
      schedule: { cron, ...(tz === undefined ? {} : { tz }) },
    };
    return [scheduled, payload];
  },
  cron<F extends Fields>(
    every: Duration,
    options: { offset?: Duration; scan?: ScanOptions<F> } = {},
  ): [Trigger, CronPayload & { batch: List<Row<F>> }] {
    const firedAt = new Expr<number>({ kind: "input", source: "payload", name: "firedAt" });
    const scan = options.scan;
    const batch = new List<Row<F>>(
      { kind: "input", source: "payload", name: "batch" },
      scan ? { kind: "row", entity: scan.entity as never } : { kind: "scalar" },
    );
    let scanSpec: CronScan | undefined;
    if (scan) {
      scanSpec = {
        entity: scan.entity.name,
        ...(scan.index === undefined ? {} : { index: scan.index }),
        prefix: (scan.prefix ?? []).map(toLiteral),
        batch: scan.batch,
      };
    }
    const offset = options.offset === undefined ? 0 : durationMs(options.offset);
    const trigger: Trigger = {
      kind: "cron",
      everyMs: durationMs(every),
      offsetMs: offset,
      ...(scanSpec ? { scan: scanSpec } : {}),
    };
    return [trigger, { firedAt, batch }];
  },
};

/** Which rows a scanning cron walks through, in batches. */
export interface ScanOptions<F extends Fields> {
  entity: Entity<F, string, string>;
  index?: string;
  /** Constant values for a prefix of the scanned key. */
  prefix?: readonly (string | number | boolean | null)[];
  /** Rows per run (1-500). */
  batch: number;
}

export type { FieldBuilder };
export { AlephBuildError };
