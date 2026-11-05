/** `app()`: the root of an Aleph backend declaration. */
import { FORMAT_VERSION, type Manifest, type Route } from "@aleph/manifest";
import { Counter, type CounterDef, Entity, type EntityDef } from "./data.js";
import type { Fields } from "./fields.js";
import { stableStringify } from "./json.js";
import { AlephBuildError } from "./recorder.js";
import { type DefaultParams, RouteBuilder } from "./route.js";

/** App-level options. */
export interface AppOptions<R extends string, S extends string> {
  /** Org member roles routes may require, for example `["owner", "admin", "member"]`. */
  roles?: readonly R[];
  /** API key scopes routes may require, for example `["projects:read"]`. */
  apiKeyScopes?: readonly S[];
}

/** An app declaration: entities, counters, and routes. `R` and `S` are its roles and scopes. */
export class App<R extends string = string, S extends string = string> {
  readonly name: string;
  readonly #options: AppOptions<R, S>;
  readonly #entities = new Map<string, Entity<Fields, string, string>>();
  readonly #counters = new Map<string, Counter<Fields, string>>();
  readonly #routes: Route[] = [];

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

  /** Declares a counter over an entity's rows. */
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
    return new RouteBuilder((route) => this.#routes.push(route), spec);
  }

  /** Builds the manifest. */
  build(): Manifest {
    const entities: Manifest["entities"] = {};
    for (const [name, entity] of this.#entities) entities[name] = entity.spec();
    const counters: Manifest["counters"] = {};
    for (const [name, counter] of this.#counters) counters[name] = counter.spec();
    return {
      formatVersion: FORMAT_VERSION,
      app: { name: this.name },
      auth: {
        roles: [...(this.#options.roles ?? [])],
        apiKeyScopes: [...(this.#options.apiKeyScopes ?? [])],
      },
      entities,
      counters,
      routes: structuredClone(this.#routes),
    };
  }

  /** The manifest as deterministic, pretty-printed JSON. */
  toJSON(): string {
    return `${stableStringify(this.build())}\n`;
  }

  #claim(map: Map<string, unknown>, kind: string, name: string): void {
    if (map.has(name))
      throw new AlephBuildError(`${kind} ${JSON.stringify(name)} is declared twice`);
  }
}

/** Declares an Aleph app. */
export function app<const R extends string = never, const S extends string = never>(
  name: string,
  options: AppOptions<R, S> = {},
): App<R, S> {
  return new App(name, options);
}
