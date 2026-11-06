/**
 * Durable workflows: steps that run one after another, each in its own bounded transaction, with
 * typed state carried between them, waits for signals with timeouts, delays, retries, and
 * compensation.
 *
 * ```ts
 * const Fulfill = a.workflow("fulfill", {
 *   input: { orderId: t.id() },
 *   state: { chargeId: t.string().max(64) },
 *   signals: { approved: { by: t.string().max(64) } },
 * });
 * Fulfill.step("charge", ($, w) => {
 *   const charge = payments.call("charge", { orderId: w.input.orderId });
 *   w.waitFor("approved", { timeout: "1d", onSignal: "ship", onTimeout: "refund", state: { chargeId: charge.id } });
 * }, { onFail: "refund" });
 * ```
 */
import type { ErrorDecl, Handler, WorkflowDecl } from "@aleph/manifest";
import {
  type Accessor,
  ErrorRef,
  type InsertValues,
  materialize,
  shapeOfFields,
  type ValuesLike,
} from "./data.js";
import { $ } from "./dsl.js";
import { Expr, type ExprLike } from "./expr.js";
import { type Fields, type FieldValue, fieldSpecs, t } from "./fields.js";
import { toNode } from "./node.js";
import { AlephBuildError, callSite, Recorder, recorder, recording } from "./recorder.js";
import { type Duration, durationMs } from "./units.js";

/** How to declare a workflow. */
export interface WorkflowDef<I extends Fields, S extends Fields, G extends Record<string, Fields>> {
  /** What an instance is started with. */
  input: I;
  /** State carried between steps. Every field starts `null`; steps set them. */
  state?: S;
  /** Signals the workflow can wait for, with their payloads. */
  signals?: G;
  /** The first step (defaults to the first step declared). */
  start?: string;
  /** The most steps one instance may run (default 100), so every instance finishes. */
  maxSteps?: number;
  /** Retries of a failed step (default 3), with exponential backoff from `backoff` (default 1s). */
  retries?: number;
  backoff?: Duration;
}

/** What a workflow step sees and how it ends. */
export interface StepContext<
  I extends Fields,
  S extends Fields,
  G extends Record<string, Fields>,
  E extends string,
> {
  /** The instance's key. */
  instanceId: Expr<string>;
  /** What the instance was started with. */
  input: { readonly [K in keyof I]: Accessor<FieldValue<I[K]>> };
  /** The current state (every field may be `null`). */
  state: { readonly [K in keyof S]: Accessor<FieldValue<S[K]> | null> };
  /** The signal that resumed this step, by name (the others are `null`). */
  signals: { readonly [K in keyof G]: Accessor<{ [F in keyof G[K]]: FieldValue<G[K][F]> } | null> };
  /** In a compensation (`onFail`) step: why the failed step failed. */
  error: Expr<string | null>;
  /** The step's declared errors. Failing with one is retried, then compensated. */
  errors: { readonly [K in E]: ErrorRef };
  /** Ends the step: run `step` next (after `delay`), updating state. */
  next(
    step: string,
    options?: { delay?: Duration | Expr<number>; state?: Partial<ValuesLike<S>> },
  ): void;
  /**
   * Ends the step: wait for `signal`, then run `onSignal` with its payload, or `onTimeout` if it
   * doesn't come within `timeout`. A signal that already arrived is used immediately.
   */
  waitFor<N extends keyof G & string>(
    signal: N,
    options: {
      timeout: Duration | Expr<number>;
      /** The step to run when the signal arrives. */
      onSignal: string;
      /** The step to run on timeout. */
      onTimeout: string;
      state?: Partial<ValuesLike<S>>;
    },
  ): void;
  /** Ends the workflow, with an optional result (readable with `get`). */
  complete(result?: unknown, options?: { state?: Partial<ValuesLike<S>> }): void;
}

/** Step options. */
export interface StepOptions<E extends string> {
  /** Errors the step may fail with. */
  errors?: Record<E, ErrorDecl>;
  /** The step to run if this one fails every retry: a compensating action (like a refund). */
  onFail?: string;
}

/** @internal Registers handlers with an app. */
export interface StepSink {
  next(): number;
  add(handler: Handler, sourceMap: Record<string, string>): void;
}

/** A workflow instance as `get` reads it. */
export type WorkflowInstance<S extends Fields> = {
  instanceId: string;
  status: "running" | "waiting" | "completed" | "failed" | "cancelled";
  step: string;
  state: { [K in keyof S]: FieldValue<S[K]> | null };
  result: unknown;
  error: string | null;
  steps: number;
};

function nodes(
  values: Record<string, unknown> | undefined,
): Record<string, ReturnType<typeof toNode>> {
  return Object.fromEntries(Object.entries(values ?? {}).map(([k, v]) => [k, toNode(v)]));
}

function delayNode(delay: Duration | Expr<number>) {
  return toNode(delay instanceof Expr ? delay : durationMs(delay));
}

/** A declared workflow. */
export class Workflow<I extends Fields, S extends Fields, G extends Record<string, Fields>> {
  readonly name: string;
  readonly #def: WorkflowDef<I, S, G>;
  readonly #sink: StepSink;
  readonly #steps: string[] = [];

  /** @internal */
  constructor(name: string, def: WorkflowDef<I, S, G>, sink: StepSink) {
    this.name = name;
    this.#def = def;
    this.#sink = sink;
  }

  /** @internal The manifest declaration. */
  spec(): WorkflowDecl {
    const start = this.#def.start ?? this.#steps[0];
    if (!start) throw new AlephBuildError(`workflow ${this.name} has no steps`);
    return {
      input: fieldSpecs(this.#def.input),
      state: fieldSpecs(this.#def.state ?? {}),
      ...(this.#def.signals
        ? {
            signals: Object.fromEntries(
              Object.entries(this.#def.signals).map(([n, f]) => [n, fieldSpecs(f)]),
            ),
          }
        : {}),
      start,
      maxSteps: this.#def.maxSteps ?? 100,
      retries: this.#def.retries ?? 3,
      backoffMs: durationMs(this.#def.backoff ?? "1s"),
    };
  }

  /** Declares a step. It ends with `w.next(...)`, `w.waitFor(...)`, or `w.complete(...)`. */
  step<E extends string = never>(
    name: string,
    fn: ($dsl: typeof $, w: StepContext<I, S, G, E>) => void,
    options: StepOptions<E> = {},
  ): this {
    if (this.#steps.includes(name)) {
      throw new AlephBuildError(
        `workflow ${this.name}: step ${JSON.stringify(name)} is declared twice`,
      );
    }
    this.#steps.push(name);
    const declared = options.errors ?? ({} as Record<E, ErrorDecl>);
    const errors = Object.fromEntries(Object.keys(declared).map((n) => [n, new ErrorRef(n)]));
    const payload = (field: string) => ({
      kind: "input" as const,
      source: "payload" as const,
      name: field,
    });
    const stateFields = this.#def.state ?? ({} as S);
    const nullable = Object.fromEntries(
      Object.entries(stateFields).map(([n, f]) => [n, f.optional()]),
    ) as Fields;
    const signals = this.#def.signals ?? ({} as G);
    const signalShapes = Object.fromEntries(
      Object.entries(signals).map(([n, fields]) => [
        n,
        { kind: "object" as const, fields: shapeOfFields(fields) },
      ]),
    );
    const ctx: StepContext<I, S, G, E> = {
      instanceId: new Expr(payload("instanceId")),
      input: materialize(
        { kind: "object", fields: shapeOfFields(this.#def.input) },
        payload("input"),
      ) as never,
      state: materialize(
        { kind: "object", fields: shapeOfFields(nullable) },
        payload("state"),
      ) as never,
      signals: materialize({ kind: "object", fields: signalShapes }, payload("signals")) as never,
      error: new Expr(payload("error")),
      errors: errors as never,
      next: (step, opts = {}) =>
        recorder().push({
          kind: "next",
          step,
          ...(opts.delay === undefined ? {} : { delayMs: delayNode(opts.delay) }),
          ...(opts.state ? { state: nodes(opts.state) } : {}),
        }),
      waitFor: (signal, opts) =>
        recorder().push({
          kind: "waitFor",
          signal,
          timeoutMs: delayNode(opts.timeout),
          // biome-ignore lint/suspicious/noThenProperty: `then` is a field of the manifest's `waitFor` statement.
          then: opts.onSignal,
          onTimeout: opts.onTimeout,
          ...(opts.state ? { state: nodes(opts.state) } : {}),
        }),
      complete: (result, opts = {}) =>
        recorder().push({
          kind: "complete",
          ...(result === undefined ? {} : { result: toNode(result) }),
          ...(opts.state ? { state: nodes(opts.state) } : {}),
        }),
    };
    const index = this.#sink.next();
    const rec = new Recorder(`handlers[${index}].plan.body`);
    const site = callSite();
    if (site) rec.sourceMap[`handlers[${index}]`] = site;
    const output = recording(rec, () => {
      fn($, ctx);
      return toNode(null);
    });
    this.#sink.add(
      {
        id: `${this.name}_${name}`,
        trigger: {
          kind: "workflow",
          workflow: this.name,
          step: name,
          ...(options.onFail ? { onFail: options.onFail } : {}),
        },
        errors: { ...declared },
        plan: { body: rec.body, output },
      },
      rec.sourceMap,
    );
    return this;
  }

  /** Starts an instance with this key unless one exists. `true` if it started. */
  start(key: ExprLike<string>, input: InsertValues<I>): Expr<boolean> {
    return new Expr(
      recorder().let({
        op: "startWorkflow",
        workflow: this.name,
        key: toNode(key),
        input: nodes(input),
      }),
    );
  }

  /** Sends a signal to an instance, after commit. */
  signal<N extends keyof G & string>(
    key: ExprLike<string>,
    signal: N,
    payload: InsertValues<G[N]>,
  ): void {
    recorder().let({
      op: "signalWorkflow",
      workflow: this.name,
      key: toNode(key),
      signal,
      payload: nodes(payload),
    });
  }

  /** Reads an instance (`null` if there's none with this key). */
  get(key: ExprLike<string>): Accessor<WorkflowInstance<S> | null> {
    const node = recorder().let({ op: "getWorkflow", workflow: this.name, key: toNode(key) });
    const state = Object.fromEntries(
      Object.entries(this.#def.state ?? {}).map(([n, f]) => [n, f.optional()]),
    ) as Fields;
    const fields = shapeOfFields({
      instanceId: t.string().max(256),
      status: t.string().max(16),
      step: t.string().max(64),
      error: t.string().max(4096),
      steps: t.int(),
    });
    return materialize(
      {
        kind: "object",
        fields: {
          ...fields,
          result: { kind: "scalar" },
          state: { kind: "object", fields: shapeOfFields(state) },
        },
      },
      node,
    ) as never;
  }

  /** Cancels a running or waiting instance, after commit. */
  cancel(key: ExprLike<string>): void {
    recorder().let({ op: "cancelWorkflow", workflow: this.name, key: toNode(key) });
  }
}
