/**
 * Entities, rows, lists, and counters: the data side of the SDK. Every declared thing owns its
 * operations (`Project.get(...)`, `row.update(...)`, `list.map(...)`), and each one records a
 * statement into the current handler's plan.
 */
import type {
  Counter as CounterSpec,
  Entity as EntitySpec,
  Expr as Node,
  Stmt,
} from "@aleph/manifest";
import { Expr, type ExprLike } from "./expr.js";
import {
  type AnyField,
  type Fields,
  type FieldValue,
  fieldSpecs,
  type RequiredFields,
} from "./fields.js";
import { type HasNode, hasNode, NODE, toNode } from "./node.js";
import { AlephBuildError, Recorder, recorder, recording } from "./recorder.js";

/** A declared error a plan can fail with. Created by `route.errors({...})`. */
export class ErrorRef {
  readonly name: string;

  constructor(name: string) {
    this.name = name;
  }
}

// ─── Shapes: how to rebuild typed accessors for a value that only exists at runtime ──────────

/** @internal */
export type Shape =
  | { kind: "scalar" }
  | { kind: "row"; entity: Entity<Fields, string, string> }
  | { kind: "object"; fields: Record<string, Shape> }
  | { kind: "list"; item: Shape };

const SCALAR: Shape = { kind: "scalar" };
const ROW_ENTITY: unique symbol = Symbol("aleph.rowEntity");

/** @internal The shape of an SDK value. */
export function shapeOf(value: unknown): Shape {
  if (value instanceof List) return { kind: "list", item: value.itemShape };
  if (typeof value === "object" && value !== null && ROW_ENTITY in value) {
    return {
      kind: "row",
      entity: (value as { [ROW_ENTITY]: Entity<Fields, string, string> })[ROW_ENTITY],
    };
  }
  if (hasNode(value) || value === null || typeof value !== "object") return SCALAR;
  if (Array.isArray(value))
    return { kind: "list", item: value.length > 0 ? shapeOf(value[0]) : SCALAR };
  const fields: Record<string, Shape> = {};
  for (const [key, v] of Object.entries(value)) fields[key] = shapeOf(v);
  return { kind: "object", fields };
}

/** @internal Builds the typed accessor for an expression of a known shape. */
export function materialize(shape: Shape, node: Node): unknown {
  switch (shape.kind) {
    case "scalar":
      return new Expr(node);
    case "row":
      return makeRow(shape.entity, node);
    case "list":
      return new List(node, shape.item);
    case "object": {
      const out: Record<string, unknown> = {};
      for (const [name, fieldShape] of Object.entries(shape.fields)) {
        out[name] = materialize(fieldShape, { kind: "field", of: node, name });
      }
      return out;
    }
  }
}

// ─── Rows ──────────────────────────────────────────────────────────────────────────────────────

/** Values for a set of fields, as expressions or constants. */
export type ValuesLike<F extends Fields> = { [K in keyof F]: ExprLike<FieldValue<F[K]>> };

/** Field values an insert takes: required fields must be given, the rest may be. */
export type InsertValues<F extends Fields> = Pick<ValuesLike<F>, RequiredFields<F>> &
  Partial<ValuesLike<F>>;

/** A row of an entity. Fields are expressions; methods record operations on the row. */
export type Row<F extends Fields> = {
  readonly [K in keyof F]: Expr<FieldValue<F[K]>>;
} & RowMethods<F> &
  HasNode;

export interface RowMethods<F extends Fields> {
  /** `false` if this row came from a `get` that found nothing. */
  exists(): Expr<boolean>;
  /** Updates fields of this row and returns the updated row. */
  update(set: Partial<ValuesLike<F>>): Row<F>;
  /** Deletes this row. */
  delete(): void;
}

const RESERVED = new Set(["exists", "update", "delete"]);

function makeRow<F extends Fields>(entity: Entity<F, string, string>, node: Node): Row<F> {
  const row: Record<string | symbol, unknown> = {
    [NODE]: node,
    [ROW_ENTITY]: entity,
    exists: () => new Expr<boolean>({ kind: "isNull", of: node }).not(),
    update: (set: Partial<ValuesLike<F>>) => entity.update(entity.keyOf(node), set),
    delete: () => entity.delete(entity.keyOf(node)),
  };
  for (const name of Object.keys(entity.fields)) {
    Object.defineProperty(row, name, {
      get: () => new Expr({ kind: "field", of: node, name }),
      enumerable: true,
    });
  }
  return row as Row<F>;
}

// ─── Lists ─────────────────────────────────────────────────────────────────────────────────────

/** A list with a statically bounded length. */
export class List<T> implements HasNode {
  readonly "~node": Node;
  /** @internal */
  readonly itemShape: Shape;

  /** @internal */
  constructor(node: Node, itemShape: Shape) {
    this["~node"] = node;
    this.itemShape = itemShape;
  }

  /** Number of items. */
  get length(): Expr<number> {
    return new Expr({ kind: "length", of: this["~node"] });
  }

  /** `true` if the list has no items. */
  isEmpty(): Expr<boolean> {
    return this.length.eq(0);
  }

  /** Transforms each item. The callback may read (get, query, ...) but never write. */
  map<R>(fn: (item: T) => R): List<R> {
    const rec = recorder();
    const item = rec.newVar();
    let result: R | undefined;
    const body = rec.block(
      () => {
        result = fn(materialize(this.itemShape, { kind: "var", id: item }) as T);
      },
      { pure: true },
    );
    const node = rec.let({ op: "map", list: this["~node"], item, body, yield: toNode(result) });
    return new List<R>(node, shapeOf(result));
  }

  /** Keeps the items for which `fn` is true. The callback may read but never write. */
  filter(fn: (item: T) => ExprLike<boolean>): List<T> {
    const rec = recorder();
    const item = rec.newVar();
    let keep: ExprLike<boolean> = false;
    const body = rec.block(
      () => {
        keep = fn(materialize(this.itemShape, { kind: "var", id: item }) as T);
      },
      { pure: true },
    );
    const node = rec.let({ op: "filter", list: this["~node"], item, body, keep: toNode(keep) });
    return new List<T>(node, this.itemShape);
  }

  /** Updates every row in a list of rows. */
  updateAll<F extends Fields>(this: List<Row<F>>, set: Partial<ValuesLike<F>>): void {
    forEach(this, (row) => {
      row.update(set);
    });
  }

  /** Deletes every row in a list of rows. */
  deleteAll<F extends Fields>(this: List<Row<F>>): void {
    forEach(this, (row) => {
      row.delete();
    });
  }
}

/** Runs `fn` once per item. Unlike `map`, the body may write. */
export function forEach<T>(list: List<T>, fn: (item: T) => void): void {
  const rec = recorder();
  const item = rec.newVar();
  const body: Stmt[] = rec.block(() =>
    fn(materialize(list.itemShape, { kind: "var", id: item }) as T),
  );
  rec.push({ kind: "forEach", list: list[NODE], item, body });
}

/** A list built from values known at build time (with expressions inside). */
export function listOf<T>(items: readonly T[]): List<T> {
  return new List<T>(toNode(items), items.length > 0 ? shapeOf(items[0]) : SCALAR);
}

// ─── Entities ──────────────────────────────────────────────────────────────────────────────────

/** Values for every primary key field. */
export type KeyValues<F extends Fields, K extends keyof F> = {
  [P in K]: ExprLike<FieldValue<F[P]>>;
};

/** How to declare an entity. */
export interface EntityDef<F extends Fields, K extends keyof F & string, I extends string> {
  /** Primary key fields, in order. A prefix of the key selects a contiguous range. */
  key: readonly K[];
  /** Fields, by name. */
  fields: F;
  /** Secondary indexes: name → indexed fields, in order. */
  indexes?: Record<I, readonly (keyof F & string)[]>;
}

/** Options for {@link Entity.query}. */
export interface QueryOptions<I extends string> {
  /** Scan this index instead of the primary key. */
  index?: I;
  /** Values for a prefix of the key (or index) fields. */
  prefix?: readonly ExprLike<unknown>[];
  /** Maximum rows. Must have a static upper bound: a constant, or an int input with a max. */
  limit: ExprLike<number>;
  /** Scan order. Defaults to ascending. */
  order?: "asc" | "desc";
}

/** A declared entity. Its methods record storage operations into the current handler. */
export class Entity<F extends Fields, K extends keyof F & string, I extends string> {
  readonly name: string;
  readonly fields: F;
  readonly #def: EntityDef<F, K, I>;

  /** @internal */
  constructor(name: string, def: EntityDef<F, K, I>) {
    for (const field of Object.keys(def.fields)) {
      if (RESERVED.has(field)) {
        throw new AlephBuildError(
          `entity ${name}: field name ${JSON.stringify(field)} is reserved for row methods`,
        );
      }
    }
    this.name = name;
    this.fields = def.fields;
    this.#def = def;
  }

  /** @internal The manifest declaration. */
  spec(): EntitySpec {
    const indexes: EntitySpec["indexes"] = {};
    for (const [name, fields] of Object.entries<readonly string[]>(this.#def.indexes ?? {})) {
      indexes[name] = { fields: [...fields] };
    }
    return { key: [...this.#def.key], fields: fieldSpecs(this.fields), indexes };
  }

  /** @internal The key map of a row expression. */
  keyOf(row: Node): KeyValues<F, K> {
    const key: Record<string, unknown> = {};
    for (const name of this.#def.key) key[name] = new Expr({ kind: "field", of: row, name });
    return key as KeyValues<F, K>;
  }

  #key(key: KeyValues<F, K>): Record<string, Node> {
    const out: Record<string, Node> = {};
    for (const [name, value] of Object.entries(key)) out[name] = toNode(value);
    return out;
  }

  /** Reads one row by primary key. Check `.exists()`: the row is `null` if it isn't found. */
  get(key: KeyValues<F, K>): Row<F> {
    return makeRow(this, recorder().let({ op: "get", entity: this.name, key: this.#key(key) }));
  }

  /** Reads one row by primary key, failing with `error` if it doesn't exist. */
  getOrFail(key: KeyValues<F, K>, error: ErrorRef): Row<F> {
    const row = this.get(key);
    recorder().push({ kind: "ensure", cond: toNode(row.exists()), error: error.name });
    return row;
  }

  /** `true` if a row with this key exists. */
  exists(key: KeyValues<F, K>): Expr<boolean> {
    return this.get(key).exists();
  }

  /** Reads several rows by primary key. Missing rows are `null` items. */
  getMany(keys: List<KeyValues<F, K>> | readonly KeyValues<F, K>[]): List<Row<F>> {
    const list = keys instanceof List ? keys : listOf(keys);
    return list.map((key) => this.get(key));
  }

  /** Inserts a row. Fails the request with 409 if the key already exists. */
  insert(values: InsertValues<F>): Row<F> {
    const nodes: Record<string, Node> = {};
    for (const [name, value] of Object.entries(values)) nodes[name] = toNode(value);
    return makeRow(this, recorder().let({ op: "insert", entity: this.name, values: nodes }));
  }

  /** Updates fields of an existing row. Fails the request with 404 if it doesn't exist. */
  update(key: KeyValues<F, K>, set: Partial<ValuesLike<F>>): Row<F> {
    const nodes: Record<string, Node> = {};
    for (const [name, value] of Object.entries(set)) nodes[name] = toNode(value);
    return makeRow(
      this,
      recorder().let({ op: "update", entity: this.name, key: this.#key(key), set: nodes }),
    );
  }

  /** Deletes a row if it exists. */
  delete(key: KeyValues<F, K>): void {
    recorder().let({ op: "delete", entity: this.name, key: this.#key(key) });
  }

  /** Reads a bounded, ordered range of rows. */
  query(options: QueryOptions<I>): List<Row<F>> {
    const node = recorder().let({
      op: "query",
      entity: this.name,
      ...(options.index === undefined ? {} : { index: options.index }),
      prefix: (options.prefix ?? []).map(toNode),
      limit: toNode(options.limit),
      ...(options.order === "desc" ? { order: "desc" as const } : {}),
    });
    return new List<Row<F>>(node, {
      kind: "row",
      entity: this as unknown as Entity<Fields, string, string>,
    });
  }
}

// ─── Counters ──────────────────────────────────────────────────────────────────────────────────

/** How to declare a counter. */
export interface CounterDef<
  F extends Fields,
  G extends keyof F & string,
  K extends keyof F & string = keyof F & string,
  I extends string = string,
> {
  /** The counted entity. */
  of: Entity<F, K, I>;
  /** Fields the count is grouped by. */
  groupBy: readonly G[];
  /** Only rows matching this predicate are counted. */
  where?: (row: Row<F>) => Expr<boolean>;
}

/** A counter: maintained on every write to its entity, read in O(1). */
export class Counter<F extends Fields, G extends keyof F & string> {
  readonly name: string;
  readonly #def: CounterDef<F, G, string, string>;

  /** @internal */
  constructor(name: string, def: CounterDef<F, G, keyof F & string, string>) {
    this.name = name;
    this.#def = def as unknown as CounterDef<F, G, string, string>;
  }

  /** @internal The manifest declaration. */
  spec(): CounterSpec {
    const spec: CounterSpec = { of: this.#def.of.name, groupBy: [...this.#def.groupBy] };
    const where = this.#def.where;
    if (where) {
      const rec = new Recorder();
      const v = rec.newVar();
      const of = this.#def.of;
      const node = recording(rec, () => toNode(where(makeRow(of, { kind: "var", id: v }))));
      if (rec.body.length > 0) {
        throw new AlephBuildError(
          `counter ${this.name}: the where predicate can't read or write data`,
        );
      }
      spec.where = { var: v, expr: node };
    }
    return spec;
  }

  /** The current count for one group. */
  get(group: { [P in G]: ExprLike<FieldValue<F[P]>> }): Expr<number> {
    const key = this.#def.groupBy.map((name) => toNode(group[name]));
    return new Expr(recorder().let({ op: "counterGet", counter: this.name, key }));
  }
}

export type { AnyField };
