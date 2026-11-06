/** `app()`: the root of an Aleph backend declaration. */
import { FORMAT_VERSION, type Handler, type Manifest, type Route } from "@aleph/manifest";
import { Counter, type CounterDef, Entity, type EntityDef, type List, type Row } from "./data.js";
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
import type { Expr } from "./expr.js";
import type { Fields } from "./fields.js";
import { stableStringify } from "./json.js";
import { AlephBuildError, callSite } from "./recorder.js";
import { type DefaultParams, type Duration, type InputValues, RouteBuilder } from "./route.js";

/** App-level options. */
export interface AppOptions<R extends string, S extends string> {
  /** Org member roles routes may require, for example `["owner", "admin", "member"]`. */
  roles?: readonly R[];
  /** API key scopes routes may require, for example `["projects:read"]`. */
  apiKeyScopes?: readonly S[];
}

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
export class App<R extends string = string, S extends string = string> {
  readonly name: string;
  readonly #options: AppOptions<R, S>;
  readonly #entities = new Map<string, Entity<Fields, string, string>>();
  readonly #counters = new Map<string, Counter<Fields, string>>();
  readonly #events = new Map<string, Spec>();
  readonly #queues = new Map<string, Spec>();
  readonly #channels = new Map<string, Spec>();
  readonly #integrations = new Map<string, Spec>();
  readonly #routes: Route[] = [];
  readonly #handlers: Handler[] = [];
  readonly #sourceMap: Record<string, string> = {};

  /** @internal */
  constructor(name: string, options: AppOptions<R, S>) {
    this.name = name;
    this.#options = options;
  }

  /** Declares an entity. */
  entity<F extends Fields, K extends keyof F & string, I extends string = never>(
    name: string,
    def: EntityDef<F, K, I>,
  ): Entity<F, K, I> {
    this.#claim(this.#entities, "entity", name);
    const entity = new Entity(name, def);
    this.#entities.set(name, entity as unknown as Entity<Fields, string, string>);
    return entity;
  }

  /** Declares a counter (or sum) over an entity's rows. */
  counter<
    F extends Fields,
    G extends keyof F & string,
    K extends keyof F & string,
    I extends string,
  >(name: string, def: CounterDef<F, G, K, I>): Counter<F, G> {
    this.#claim(this.#counters, "counter", name);
    const counter = new Counter<F, G>(name, def as unknown as CounterDef<F, G>);
    this.#counters.set(name, counter as unknown as Counter<Fields, string>);
    return counter;
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
   * Runs `fn` on a schedule: every `interval` (`"5m"`, `"1d"`, ...), optionally offset. With
   * `scan`, it runs once per batch of an entity's rows instead, each batch separately bounded.
   */
  every<F extends Fields = Fields, E extends string = never>(
    id: string,
    interval: Duration,
    fn: HandlerFn<CronPayload & { batch: List<Row<F>> }, E>,
    options: HandlerOptions<E> & { offset?: Duration; scan?: ScanOptions<F> } = {},
  ): void {
    const cron = {
      ...(options.offset === undefined ? {} : { offset: options.offset }),
      ...(options.scan === undefined ? {} : { scan: options.scan }),
    };
    const [trigger, payload] = triggers.cron(interval, cron);
    recordHandler(this.#handlerSink(), id, trigger, payload, fn, options);
  }

  /** Builds the manifest. */
  build(options: BuildOptions = {}): Manifest {
    const specs = <T>(map: Map<string, Spec>) =>
      Object.fromEntries([...map].map(([name, v]) => [name, v.spec() as T]));
    const manifest: Manifest = {
      formatVersion: FORMAT_VERSION,
      app: { name: this.name },
      auth: {
        roles: [...(this.#options.roles ?? [])],
        apiKeyScopes: [...(this.#options.apiKeyScopes ?? [])],
      },
      entities: Object.fromEntries([...this.#entities].map(([n, e]) => [n, e.spec()])),
      counters: Object.fromEntries([...this.#counters].map(([n, c]) => [n, c.spec()])),
      routes: structuredClone(this.#routes),
    };
    if (this.#events.size > 0) manifest.events = specs(this.#events);
    if (this.#queues.size > 0) manifest.queues = specs(this.#queues);
    if (this.#channels.size > 0) manifest.channels = specs(this.#channels);
    if (this.#integrations.size > 0) manifest.integrations = specs(this.#integrations);
    if (this.#handlers.length > 0) manifest.handlers = structuredClone(this.#handlers);
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
export function app<const R extends string = never, const S extends string = never>(
  name: string,
  options: AppOptions<R, S> = {},
): App<R, S> {
  return new App(name, options);
}
