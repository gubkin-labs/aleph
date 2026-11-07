/** `app()`: the root of an Aleph backend declaration. */
import {
  type Cors,
  FORMAT_VERSION,
  type Handler,
  type Manifest,
  type Route,
  type Verify,
} from "@aleph-lang/manifest";
import { type BlobInfoFields, Bucket, type BucketDef, uploadPayload } from "./blobs.js";
import {
  type Accessor,
  type ComputedGroup,
  Counter,
  type CounterAggDef,
  type CounterDef,
  type CounterValue,
  Entity,
  type EntityDef,
  type List,
  materialize,
  type Row,
  shapeOfFields,
} from "./data.js";
import type { $ } from "./dsl.js";
import {
  type ChangePayload,
  Channel,
  type CronPayload,
  Event,
  type HandlerCtx,
  type HandlerOptions,
  type IntegrationDef,
  IntegrationHandle,
  type OperationDef,
  Queue,
  type QueueOptions,
  recordHandler,
  type ScanOptions,
  triggers,
} from "./effects.js";
import { Expr, type ExprLike } from "./expr.js";
import { type Fields, type FieldValue, fieldSpecs, t } from "./fields.js";
import { stableStringify } from "./json.js";
import { toNode } from "./node.js";
import { AlephBuildError, callSite, recorder } from "./recorder.js";
import { type DefaultParams, type InputValues, RouteBuilder } from "./route.js";
import { type Duration, durationMs } from "./units.js";
import { Workflow, type WorkflowDef } from "./workflow.js";

/** App-level options. */
export interface AppOptions<R extends string, S extends string, P extends Fields = Fields> {
  /** Org member roles routes may require, for example `["owner", "admin", "member"]`. */
  roles?: readonly R[];
  /** API key scopes routes may require, for example `["projects:read"]`. */
  apiKeyScopes?: readonly S[];
  /**
   * User profile fields the app reads (`email`, `name`, custom claims), resolved by the
   * platform. Read the caller's with `a.caller.profile`, anyone's with `a.users.get(id)`.
   */
  profile?: P;
  /** Cross-origin access for browsers. */
  cors?: {
    origins: readonly string[];
    credentials?: boolean;
    headers?: readonly string[];
    maxAge?: Duration;
  };
}

/** Options for {@link App.webhook}. */
export interface WebhookOptions {
  /** How deliveries are signed: {@link verify}.hmacSha256, .stripe, or .github. */
  verify: Verify;
  /** Run each event once: where its id is, and how long ids are remembered (default 3 days). */
  dedupe?: { header?: string; bodyField?: string; ttl?: Duration };
}

/** Webhook signature schemes. `secret` names a secret the platform holds. */
export const verify = {
  /** HMAC-SHA256 of the raw body (or `timestamp.body`, with `timestampHeader`). */
  hmacSha256: (options: {
    secret: string;
    header: string;
    encoding?: "hex" | "base64";
    prefix?: string;
    timestampHeader?: string;
    tolerance?: Duration;
  }): Verify => ({
    kind: "hmacSha256",
    secret: options.secret,
    header: options.header.toLowerCase(),
    encoding: options.encoding ?? "hex",
    ...(options.prefix === undefined ? {} : { prefix: options.prefix }),
    ...(options.timestampHeader === undefined
      ? {}
      : { timestampHeader: options.timestampHeader.toLowerCase() }),
    ...(options.tolerance === undefined
      ? {}
      : { toleranceS: Math.round(durationMs(options.tolerance) / 1000) }),
  }),
  /** Stripe's `Stripe-Signature` (5-minute tolerance by default). */
  stripe: (secret: string, options: { tolerance?: Duration } = {}): Verify => ({
    kind: "stripe",
    secret,
    ...(options.tolerance === undefined
      ? {}
      : { toleranceS: Math.round(durationMs(options.tolerance) / 1000) }),
  }),
  /** GitHub's `X-Hub-Signature-256`. */
  github: (secret: string): Verify => ({ kind: "github", secret }),
};

/** The names of a counter's group parts: field names and computed parts' names. */
export type GroupNames<GB> = GB extends readonly (infer P)[]
  ? P extends string
    ? P
    : P extends { name: infer N extends string }
      ? N
      : never
  : never;

/** A user from the directory: their id and declared profile fields (each may be `null`). */
export type UserValue<P extends Fields> = { userId: string } & {
  [K in keyof P]: FieldValue<P[K]> | null;
};

/** Plain-language schedules for {@link App.every}, converted to cron expressions. */
const WEEKDAY = /^(mon|tue|wed|thu|fri|sat|sun|daily|weekdays|weekends) (\d{1,2}):(\d{2})$/;

/** Options for {@link App.build}. */
export interface BuildOptions {
  /**
   * Include a source map from manifest paths to `file:line:col`, so diagnostics point at your
   * code. Off by default, because locations depend on where the build runs.
   */
  sourceMap?: boolean;
}

type HandlerFn<P, E extends string> = ($dsl: typeof $, ctx: HandlerCtx<P, E>) => unknown;

interface Spec {
  spec(): unknown;
}

/** An app declaration. `R` and `S` are its declared roles and API key scopes. */
export class App<R extends string = string, S extends string = string, P extends Fields = Fields> {
  readonly name: string;
  readonly #options: AppOptions<R, S, P>;
  readonly #entities = new Map<string, Entity<Fields, string, string>>();
  readonly #counters = new Map<string, Counter<Fields, string>>();
  readonly #events = new Map<string, Spec>();
  readonly #queues = new Map<string, Spec>();
  readonly #channels = new Map<string, Spec>();
  readonly #buckets = new Map<string, Spec>();
  readonly #integrations = new Map<string, Spec>();
  readonly #workflows = new Map<string, Spec>();
  readonly #routes: Route[] = [];
  readonly #handlers: Handler[] = [];
  readonly #sourceMap: Record<string, string> = {};

  /** @internal */
  constructor(name: string, options: AppOptions<R, S, P>) {
    this.name = name;
    this.#options = options;
  }

  /** The signed-in caller: their id and declared profile fields (`null` when anonymous). */
  get caller(): {
    id: Expr<string>;
    profile: { readonly [K in keyof P]: Accessor<FieldValue<P[K]> | null> };
  } {
    const profile = Object.fromEntries(
      Object.keys(this.#options.profile ?? {}).map((field) => [
        field,
        new Expr({ kind: "callerProfile", field }),
      ]),
    );
    return { id: new Expr({ kind: "caller" }), profile: profile as never };
  }

  /** The tenant's user directory. */
  get users(): {
    /** A user's profile, or `null` if there's no such user (check `.userId.isSet()`). */
    get(userId: ExprLike<string>): Accessor<UserValue<P> | null>;
  } {
    const fields = { userId: t.string().max(256), ...(this.#options.profile ?? {}) };
    return {
      get: (userId) =>
        materialize(
          { kind: "object", fields: shapeOfFields(fields) },
          recorder().let({ op: "getUser", userId: toNode(userId) }),
        ) as never,
    };
  }

  /** Declares an entity. */
  entity<F extends Fields, K extends keyof F & string, I extends string = never>(
    name: string,
    def: EntityDef<F, K, I>,
  ): Entity<F, K, I> {
    this.#claim(this.#entities, "entity", name);
    const entity = new Entity(name, def);
    this.#entities.set(name, entity as unknown as Entity<Fields, string, string>);
    if (def.parent?.onDelete === "cascade") this.#cascade(entity as never);
    return entity;
  }

  /**
   * Deleting a parent row deletes its children: a delete trigger enqueues a job that deletes
   * them in batches of 50, re-enqueuing itself until none are left. Each batch is a separately
   * bounded job.
   */
  #cascade(child: Entity<Fields, string, string>): void {
    const parent = child.parent;
    if (!parent) return;
    const keyFields = child.keyFields.filter((f) => parent.key[f] !== undefined);
    const payload = Object.fromEntries(keyFields.map((f) => [f, child.fields[f]])) as Fields;
    const queue = this.queue(`cascade_${child.name}`, { payload });
    const prefix = (values: Record<string, unknown>) => keyFields.map((f) => values[f]);
    this.onQueue(`cascade_${child.name}_batch`, queue, ($, { payload: values }) => {
      const batch = child.query({ prefix: prefix(values as never) as never, limit: 50 });
      $.forEach(batch, (row) => row.delete());
      $.when(batch.length.eq(50), () => queue.enqueue(values as never));
    });
    this.onDelete(`cascade_${child.name}`, parent.entity as never, (_, { payload: change }) => {
      const before = change.before as unknown as Record<string, unknown>;
      queue.enqueue(
        Object.fromEntries(keyFields.map((f) => [f, before[parent.key[f] as string]])) as never,
      );
    });
  }

  /** Declares a counter over an entity's rows: a count, a sum, a min/max, or a distinct count. */
  counter<
    F extends Fields,
    K extends keyof F & string,
    I extends string,
    const GB extends readonly ((keyof F & string) | ComputedGroup<F>)[],
    const A extends CounterAggDef<F> | undefined = undefined,
  >(
    name: string,
    def: Omit<CounterDef<F, keyof F & string, K, I, string>, "groupBy" | "agg"> & {
      groupBy: GB;
      agg?: A;
    },
  ): Counter<F, GroupNames<GB>, CounterValue<F, A>> {
    this.#claim(this.#counters, "counter", name);
    const counter = new Counter<F, GroupNames<GB>, CounterValue<F, A>>(name, def as never);
    this.#counters.set(name, counter as unknown as Counter<Fields, string>);
    return counter;
  }

  /**
   * An inbound webhook: `a.webhook("POST /webhooks/stripe", { verify: verify.stripe("STRIPE") })`.
   * The signature is checked on the raw body before the plan runs (401 if it doesn't match),
   * undeclared body fields are ignored, and with `dedupe` each event runs once.
   */
  webhook<const Spec extends string>(
    spec: Spec,
    options: WebhookOptions,
  ): RouteBuilder<
    DefaultParams<Spec extends `${string} ${infer Path}` ? Path : never>,
    Record<never, never>,
    Record<never, never>,
    never,
    R,
    S
  > {
    const dedupe = options.dedupe;
    return this.route(spec)
      .auth("public")
      .webhookConfig({
        verify: options.verify,
        ...(dedupe
          ? {
              dedupe: {
                ...(dedupe.header ? { header: dedupe.header.toLowerCase() } : {}),
                ...(dedupe.bodyField ? { bodyField: dedupe.bodyField } : {}),
                ttlMs: durationMs(dedupe.ttl ?? "3d"),
              },
            }
          : {}),
      });
  }

  /** Declares an event with a typed payload. */
  event<P extends Fields>(name: string, def: { payload: P }): Event<P> {
    this.#claim(this.#events, "event", name);
    const event = new Event(name, def.payload);
    this.#events.set(name, event);
    return event;
  }

  /** Declares a queue. Give it a handler with {@link App.onQueue}. */
  queue<P extends Fields>(name: string, options: QueueOptions<P>): Queue<P> {
    this.#claim(this.#queues, "queue", name);
    const queue = new Queue(name, options);
    this.#queues.set(name, queue);
    return queue;
  }

  /** Declares a realtime channel. */
  channel<K extends Fields, M extends Fields>(
    name: string,
    def: { key: K; message: M },
  ): Channel<K, M> {
    this.#claim(this.#channels, "channel", name);
    const channel = new Channel(name, def);
    this.#channels.set(name, channel);
    return channel;
  }

  /**
   * Declares a bucket of files. Plans hand out signed URLs (`uploadUrl`, `downloadUrl`) and read
   * or delete files' metadata; the bytes go straight between the client and the storage.
   */
  bucket(name: string, def: BucketDef): Bucket {
    this.#claim(this.#buckets, "bucket", name);
    const bucket = new Bucket(name, def);
    this.#buckets.set(name, bucket);
    return bucket;
  }

  /** Declares an integration with typed operations. */
  integration<const O extends Record<string, OperationDef>>(
    name: string,
    def: IntegrationDef<O>,
  ): IntegrationHandle<O> {
    this.#claim(this.#integrations, "integration", name);
    const integration = new IntegrationHandle(name, def);
    this.#integrations.set(name, integration);
    return integration;
  }

  /**
   * Declares a durable workflow. Add steps with `.step(name, fn)`; start instances from routes
   * or handlers with `.start(key, input)`.
   */
  workflow<
    I extends Fields,
    S extends Fields = Record<never, never>,
    G extends Record<string, Fields> = Record<never, never>,
  >(name: string, def: WorkflowDef<I, S, G>): Workflow<I, S, G> {
    this.#claim(this.#workflows, "workflow", name);
    const workflow = new Workflow(name, def, this.#handlerSink());
    this.#workflows.set(name, workflow);
    return workflow;
  }

  /** Starts a route: `a.route("GET /orgs/:orgId/projects")`. */
  route<const Spec extends string>(
    spec: Spec,
  ): RouteBuilder<
    DefaultParams<Spec extends `${string} ${infer P}` ? P : never>,
    Record<never, never>,
    Record<never, never>,
    never,
    R,
    S
  > {
    const sink = {
      next: () => this.#routes.length,
      add: (route: Route, sourceMap: Record<string, string>) => {
        this.#routes.push(route);
        Object.assign(this.#sourceMap, sourceMap);
      },
    };
    return new RouteBuilder(sink, spec);
  }

  #handlerSink() {
    return {
      next: () => this.#handlers.length,
      add: (handler: Handler, sourceMap: Record<string, string>) => {
        if (this.#handlers.some((h) => h.id === handler.id)) {
          throw new AlephBuildError(`handler ${JSON.stringify(handler.id)} is declared twice`);
        }
        this.#handlers.push(handler);
        Object.assign(this.#sourceMap, sourceMap);
      },
    };
  }

  /** Runs `fn` after each emitted `event`. */
  onEvent<P extends Fields, E extends string = never>(
    id: string,
    event: Event<P>,
    fn: HandlerFn<InputValues<P>, E>,
    options?: HandlerOptions<E>,
  ): void {
    const [trigger, payload] = triggers.event(event);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /** Runs `fn` after each committed insert into `entity`. */
  onInsert<F extends Fields, E extends string = never>(
    id: string,
    entity: Entity<F, keyof F & string, string>,
    fn: HandlerFn<ChangePayload<F>, E>,
    options?: HandlerOptions<E>,
  ): void {
    this.#onChange(id, entity, "inserted", fn, options);
  }

  /** Runs `fn` after each committed update of `entity`. */
  onUpdate<F extends Fields, E extends string = never>(
    id: string,
    entity: Entity<F, keyof F & string, string>,
    fn: HandlerFn<ChangePayload<F>, E>,
    options?: HandlerOptions<E>,
  ): void {
    this.#onChange(id, entity, "updated", fn, options);
  }

  /** Runs `fn` after each committed delete from `entity`. */
  onDelete<F extends Fields, E extends string = never>(
    id: string,
    entity: Entity<F, keyof F & string, string>,
    fn: HandlerFn<ChangePayload<F>, E>,
    options?: HandlerOptions<E>,
  ): void {
    this.#onChange(id, entity, "deleted", fn, options);
  }

  #onChange<F extends Fields, E extends string>(
    id: string,
    entity: Entity<F, keyof F & string, string>,
    change: "inserted" | "updated" | "deleted",
    fn: HandlerFn<ChangePayload<F>, E>,
    options: HandlerOptions<E> | undefined,
  ): void {
    const [trigger, payload] = triggers.change(
      entity as unknown as Entity<F, string, string>,
      change,
    );
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /** Runs `fn` after each upload to `bucket` finishes, with the file's metadata. */
  onUpload<E extends string = never>(
    id: string,
    bucket: Bucket,
    fn: HandlerFn<InputValues<BlobInfoFields>, E>,
    options?: HandlerOptions<E>,
  ): void {
    recordHandler(
      this.#handlerSink(),
      id,
      { kind: "upload", bucket: bucket.name },
      uploadPayload(),
      fn,
      options,
    );
  }

  /** Runs `fn` for each job on `queue`. A failed job is retried with backoff. */
  onQueue<P extends Fields, E extends string = never>(
    id: string,
    queue: Queue<P>,
    fn: HandlerFn<InputValues<P>, E>,
    options?: HandlerOptions<E>,
  ): void {
    const [trigger, payload] = triggers.queue(queue);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /** Runs `fn` for each job on `queue` that failed all of its retries. */
  onDeadLetter<P extends Fields, E extends string = never>(
    id: string,
    queue: Queue<P>,
    fn: HandlerFn<InputValues<P> & { error: Expr<string>; attempts: Expr<number> }, E>,
    options?: HandlerOptions<E>,
  ): void {
    const [trigger, payload] = triggers.deadLetter(queue);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /**
   * Runs `fn` on a schedule: every `interval` (`"5m"`, `"1d"`, ...), optionally offset, or at
   * calendar times like `"mon 09:00"`, `"daily 18:30"`, or `"weekdays 08:00"` in `tz`. With
   * `scan`, it runs once per batch of an entity's rows instead, each batch separately bounded.
   */
  every<F extends Fields = Fields, E extends string = never>(
    id: string,
    interval:
      | Duration
      | `${"mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun" | "daily" | "weekdays" | "weekends"} ${string}`,
    fn: HandlerFn<CronPayload & { batch: List<Row<F>> }, E>,
    options: HandlerOptions<E> & { offset?: Duration; scan?: ScanOptions<F>; tz?: string } = {},
  ): void {
    const calendar = typeof interval === "string" ? WEEKDAY.exec(interval) : null;
    if (calendar) {
      const [, day, hour, minute] = calendar;
      const days: Record<string, string> = { daily: "*", weekdays: "1-5", weekends: "0,6" };
      const cron = `${Number(minute)} ${Number(hour)} * * ${days[day as string] ?? day}`;
      this.cron(id, cron, fn, options);
      return;
    }
    const cron = {
      ...(options.offset === undefined ? {} : { offset: options.offset }),
      ...(options.scan === undefined ? {} : { scan: options.scan }),
    };
    const [trigger, payload] = triggers.cron(interval as Duration, cron);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /**
   * Runs `fn` on a cron schedule (`minute hour day month weekday`, like `"0 9 * * mon"`) in a
   * time zone (default UTC), across daylight-saving changes.
   */
  cron<F extends Fields = Fields, E extends string = never>(
    id: string,
    expression: string,
    fn: HandlerFn<CronPayload & { batch: List<Row<F>> }, E>,
    options: HandlerOptions<E> & { scan?: ScanOptions<F>; tz?: string } = {},
  ): void {
    const [trigger, payload] = triggers.schedule(expression, options.tz, options.scan);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /** Builds the manifest. */
  build(options: BuildOptions = {}): Manifest {
    const specs = <T>(map: Map<string, Spec>) =>
      Object.fromEntries([...map].map(([name, v]) => [name, v.spec() as T]));
    const profile = this.#options.profile;
    const manifest: Manifest = {
      formatVersion: FORMAT_VERSION,
      app: { name: this.name },
      auth: {
        roles: [...(this.#options.roles ?? [])],
        apiKeyScopes: [...(this.#options.apiKeyScopes ?? [])],
        ...(profile && Object.keys(profile).length > 0 ? { profile: fieldSpecs(profile) } : {}),
      },
      entities: Object.fromEntries([...this.#entities].map(([n, e]) => [n, e.spec()])),
      counters: Object.fromEntries([...this.#counters].map(([n, c]) => [n, c.spec()])),
      routes: structuredClone(this.#routes),
    };
    if (this.#events.size > 0) manifest.events = specs(this.#events);
    if (this.#queues.size > 0) manifest.queues = specs(this.#queues);
    if (this.#channels.size > 0) manifest.channels = specs(this.#channels);
    if (this.#buckets.size > 0) manifest.buckets = specs(this.#buckets);
    if (this.#integrations.size > 0) manifest.integrations = specs(this.#integrations);
    if (this.#workflows.size > 0) manifest.workflows = specs(this.#workflows);
    if (this.#handlers.length > 0) manifest.handlers = structuredClone(this.#handlers);
    const cors = this.#options.cors;
    if (cors) {
      const spec: Cors = { origins: [...cors.origins] };
      if (cors.credentials) spec.credentials = true;
      if (cors.headers?.length) spec.headers = cors.headers.map((h) => h.toLowerCase());
      if (cors.maxAge !== undefined) spec.maxAgeS = Math.round(durationMs(cors.maxAge) / 1000);
      manifest.cors = spec;
    }
    if (options.sourceMap && Object.keys(this.#sourceMap).length > 0) {
      manifest.sourceMap = { ...this.#sourceMap };
    }
    return manifest;
  }

  /** The manifest as deterministic, pretty-printed JSON. */
  toJSON(options: BuildOptions = {}): string {
    return `${stableStringify(this.build(options))}\n`;
  }

  #claim(map: Map<string, unknown>, kind: string, name: string): void {
    if (map.has(name)) {
      throw new AlephBuildError(`${kind} ${JSON.stringify(name)} is declared twice`);
    }
    const site = callSite();
    const section = kind === "entity" ? "entities" : `${kind}s`;
    if (site) this.#sourceMap[`${section}.${name}`] = site;
  }
}

/** Declares an Aleph app. */
export function app<
  const R extends string = never,
  const S extends string = never,
  const P extends Fields = Record<never, never>,
>(name: string, options: AppOptions<R, S, P> = {}): App<R, S, P> {
  return new App(name, options);
}
