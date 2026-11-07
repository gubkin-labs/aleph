/**
 * Entities, rows, lists, and counters: the data side of the SDK. Every declared thing owns its
 * operations (`Project.get(...)`, `row.update(...)`, `list.map(...)`), and each one records a
 * statement into the current handler's plan.
 */
import type {
  AggOp,
  Counter as CounterSpec,
  Entity as EntitySpec,
  FieldType,
  IndexPart,
  Expr as Node,
  Stmt,
} from "@aleph-lang/manifest";
import { Expr, type ExprLike, type Numeric } from "./expr.js";
import {
  type AnyField,
  type Fields,
  type FieldValue,
  fieldSpecs,
  type RequiredFields,
} from "./fields.js";
import { type HasNode, hasNode, NODE, toNode } from "./node.js";
import { AlephBuildError, Recorder, recorder, recording } from "./recorder.js";
import { type Duration, durationMs } from "./units.js";

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
const SHAPE: unique symbol = Symbol("aleph.shape");

/** @internal The shape of a value of a declared field type. */
export function shapeOfType(type: FieldType): Shape {
  switch (type.kind) {
    case "array":
      return { kind: "list", item: shapeOfType(type.items) };
    case "object":
      return {
        kind: "object",
        fields: Object.fromEntries(
          Object.entries(type.fields).map(([name, f]) => [name, shapeOfType(f.type)]),
        ),
      };
    default:
      return SCALAR;
  }
}

/** @internal The shapes of a record of declared fields. */
export function shapeOfFields(fields: Fields): Record<string, Shape> {
  return Object.fromEntries(
    Object.entries(fields).map(([name, f]) => [name, shapeOfType(f.spec.type)]),
  );
}

/** @internal The shape of an SDK value. */
export function shapeOf(value: unknown): Shape {
  if (typeof value === "object" && value !== null && SHAPE in value) {
    return (value as { [SHAPE]: Shape })[SHAPE];
  }
  if (value instanceof List) return { kind: "list", item: value.itemShape };
  if (value instanceof Page) {
    return {
      kind: "object",
      fields: { items: { kind: "list", item: value.items.itemShape }, cursor: SCALAR },
    };
  }
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
      const out: Record<string | symbol, unknown> = {};
      for (const [name, fieldShape] of Object.entries(shape.fields)) {
        out[name] = materialize(fieldShape, { kind: "field", of: node, name });
      }
      // The object stands for one expression (which may be `null`), not a rebuilt literal.
      Object.defineProperty(out, NODE, { value: node, enumerable: false });
      Object.defineProperty(out, SHAPE, { value: shape, enumerable: false });
      return out;
    }
  }
}

/**
 * What a plan sees for a value of type `V`: lists of bounded length become {@link List}s,
 * objects become objects of accessors, and everything else is an {@link Expr}.
 */
export type Accessor<V> = [NonNullable<V>] extends [readonly (infer U)[]]
  ? List<Accessor<U>>
  : [NonNullable<V>] extends [Record<string, unknown>]
    ? ObjectAccessor<NonNullable<V>, null extends V ? true : false>
    : Expr<V>;

/** An object value's fields. If the object may be `null`, every field may be too. */
export type ObjectAccessor<O, N extends boolean> = {
  readonly [K in keyof O]: Accessor<N extends true ? O[K] | null : O[K]>;
} & HasNode;

/**
 * A value accepted where a value of type `V` is expected: an expression, a constant, or (for
 * lists and objects) a literal built from those.
 */
export type ValueLike<V> = Expr<V> | PlainLike<V>;

type PlainLike<V> = V extends readonly (infer U)[]
  ? readonly ValueLike<U>[] | List<unknown>
  : V extends Record<string, unknown>
    ? { readonly [K in keyof V]: ValueLike<V[K]> }
    : V;

// ─── Rows ──────────────────────────────────────────────────────────────────────────────────────

/** Values for a set of fields, as expressions or constants. */
export type ValuesLike<F extends Fields> = { [K in keyof F]: ValueLike<FieldValue<F[K]>> };

/** Field values an insert takes: required fields must be given, the rest may be. */
export type InsertValues<F extends Fields> = Pick<ValuesLike<F>, RequiredFields<F>> &
  Partial<ValuesLike<F>>;

/** A row of an entity. Fields are expressions; methods record operations on the row. */
export type Row<F extends Fields> = {
  readonly [K in keyof F]: Accessor<FieldValue<F[K]>>;
} & RowMethods<F> &
  HasNode;

export interface RowMethods<F extends Fields> {
  /** `false` if this row came from a `get` that found nothing. */
  exists(): Expr<boolean>;
  /** Updates fields of this row and returns the updated row. */
  update(set: Partial<ValuesLike<F>>, options?: UpdateOptions): Row<F>;
  /** Deletes this row. */
  delete(): void;
}

/** Options for updates. */
export interface UpdateOptions {
  /** For versioned entities: fail with 409 `version_conflict` unless the row has this version. */
  ifVersion?: ExprLike<number | null>;
}

const RESERVED = new Set(["exists", "update", "delete"]);

function makeRow<F extends Fields>(entity: Entity<F, string, string>, node: Node): Row<F> {
  const row: Record<string | symbol, unknown> = {
    [NODE]: node,
    [ROW_ENTITY]: entity,
    exists: () => new Expr<boolean>({ kind: "isNull", of: node }).not(),
    update: (set: Partial<ValuesLike<F>>, options?: UpdateOptions) =>
      entity.update(entity.keyOf(node) as never, set, options),
    delete: () => entity.delete(entity.keyOf(node) as never),
  };
  for (const [name, field] of Object.entries(entity.fields)) {
    const shape = shapeOfType(field.spec.type);
    Object.defineProperty(row, name, {
      get: () => materialize(shape, { kind: "field", of: node, name }),
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

  /** The first item, or `null` if the list is empty. */
  first(): T {
    return materialize(this.itemShape, { kind: "first", of: this["~node"] }) as T;
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
      { pure: true, suffix: ".op.body" },
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
      { pure: true, suffix: ".op.body" },
    );
    const node = rec.let({ op: "filter", list: this["~node"], item, body, keep: toNode(keep) });
    return new List<T>(node, this.itemShape);
  }

  #agg<R>(op: AggOp, fn?: (item: T) => unknown): Expr<R> {
    if (!fn) return new Expr({ kind: "agg", op, list: this["~node"], item: freshVar() });
    const item = freshVar();
    const value = toNode(fn(materialize(this.itemShape, { kind: "var", id: item }) as T));
    return new Expr({ kind: "agg", op, list: this["~node"], item, value });
  }

  /** The sum of the items (or of `fn(item)`); 0 for an empty list. */
  sum<N extends Numeric>(fn: (item: T) => ExprLike<N>): Expr<N>;
  sum<N extends Numeric>(this: List<Expr<N | null>>): Expr<N>;
  sum(fn?: (item: T) => unknown): Expr<Numeric> {
    return this.#agg("sum", fn);
  }

  /** The smallest item (or `fn(item)`), or `null` for an empty list. */
  min<V>(fn: (item: T) => ExprLike<V>): Expr<V | null>;
  min<V>(this: List<Expr<V>>): Expr<V | null>;
  min(fn?: (item: T) => unknown): Expr<unknown> {
    return this.#agg("min", fn);
  }

  /** The largest item (or `fn(item)`), or `null` for an empty list. */
  max<V>(fn: (item: T) => ExprLike<V>): Expr<V | null>;
  max<V>(this: List<Expr<V>>): Expr<V | null>;
  max(fn?: (item: T) => unknown): Expr<unknown> {
    return this.#agg("max", fn);
  }

  /** `true` if `fn` holds for any item. */
  some(fn: (item: T) => ExprLike<boolean>): Expr<boolean> {
    return this.#agg("some", fn);
  }

  /** `true` if `fn` holds for every item (also for an empty list). */
  every(fn: (item: T) => ExprLike<boolean>): Expr<boolean> {
    return this.#agg("every", fn);
  }

  /** How many items `fn` holds for (or all items). */
  count(fn?: (item: T) => ExprLike<boolean>): Expr<number> {
    return this.#agg("count", fn);
  }

  /** The items sorted by a key (a string, number, decimal, or bool), stable. */
  sortBy(fn: (item: T) => Expr<unknown>, order: "asc" | "desc" = "asc"): List<T> {
    const item = freshVar();
    const by = toNode(fn(materialize(this.itemShape, { kind: "var", id: item }) as T));
    const node: Node = {
      kind: "sort",
      list: this["~node"],
      item,
      by,
      ...(order === "desc" ? { desc: true } : {}),
    };
    return new List<T>(node, this.itemShape);
  }

  /** Items from `start` up to `end`, like JavaScript's `slice`. */
  slice(start: ExprLike<number>, end?: ExprLike<number>): List<T> {
    const node: Node = {
      kind: "slice",
      of: this["~node"],
      start: toNode(start),
      ...(end === undefined ? {} : { end: toNode(end) }),
    };
    return new List<T>(node, this.itemShape);
  }

  /** The first `n` items. With a constant `n`, the list's bound shrinks to it. */
  take(n: ExprLike<number>): List<T> {
    return this.slice(0, n);
  }

  /** The list without repeated items (first occurrences kept). */
  distinct(): List<T> {
    return new List<T>({ kind: "distinct", of: this["~node"] }, this.itemShape);
  }

  /** This list followed by others. */
  concat(...others: List<T>[]): List<T> {
    const parts = [this["~node"], ...others.map((o) => o["~node"])];
    return new List<T>({ kind: "listConcat", parts }, this.itemShape);
  }

  /** `true` if the list contains `value`. */
  includes(value: unknown): Expr<boolean> {
    return new Expr({ kind: "includes", list: this["~node"], value: toNode(value) });
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

/** A fresh variable id: from the current plan, or a row expression's recorder. */
function freshVar(): number {
  return recorder().newVar();
}

/** Runs `fn` once per item. Unlike `map`, the body may write. */
export function forEach<T>(list: List<T>, fn: (item: T) => void): void {
  const rec = recorder();
  const item = rec.newVar();
  const body: Stmt[] = rec.block(
    () => fn(materialize(list.itemShape, { kind: "var", id: item }) as T),
    { suffix: ".body" },
  );
  rec.push({ kind: "forEach", list: list[NODE], item, body });
}

/**
 * What a plan sees for a value it was given: SDK values stay as they are, and plain numbers,
 * strings, and booleans become expressions.
 */
export type Lifted<V> = V extends number
  ? Expr<number>
  : V extends string
    ? Expr<string>
    : V extends boolean
      ? Expr<boolean>
      : V;

/** The state a {@link loop} carries: each field as the plan sees it. */
export type LoopState<S> = { [K in keyof S]: Lifted<S[K]> };

/** Binds a value to a variable, so later steps share it instead of recomputing it. */
export function bind<V>(value: V): Lifted<V> {
  const node = recorder().let({ op: "eval", expr: toNode(value) });
  return materialize(shapeOf(value), node) as Lifted<V>;
}

/** Options for {@link loop}. */
export interface LoopOptions<S> {
  /** The most passes (1-1000). The checker counts the loop as this many passes. */
  max: number;
  /** The initial state: named values the loop carries from one pass to the next. */
  state: S;
  /**
   * Checked before each pass, on the current state; the loop stops when it's false. It may only
   * compute from the state: do reads in the body and carry their results in the state.
   */
  while?: (state: LoopState<S>) => ExprLike<boolean>;
}

/**
 * A bounded loop that carries state. Starting from `state`, each pass checks `while`, runs
 * `fn` on the current state, and takes what it returns as the next state, for at most `max`
 * passes. Returns the final state. Like `map`, the body may read but never write.
 */
export function loop<S extends Record<string, unknown>>(
  options: LoopOptions<S>,
  fn: (state: LoopState<S>) => { [K in keyof S]: unknown },
): LoopState<S> {
  if (!Number.isInteger(options.max) || options.max < 1 || options.max > 1000) {
    throw new AlephBuildError(`a loop's max must be an integer from 1 to 1000, got ${options.max}`);
  }
  const rec = recorder();
  const shape = shapeOf(options.state);
  const v = rec.newVar();
  const current = materialize(shape, { kind: "var", id: v }) as LoopState<S>;
  let cond: Node | undefined;
  const condition = options.while;
  if (condition) {
    const recorded = rec.block(
      () => {
        cond = toNode(condition(current));
      },
      { pure: true, suffix: ".op.while" },
    );
    if (recorded.length > 0) {
      throw new AlephBuildError(
        "a loop's while condition can only compute from the state: do reads in the body and " +
          "carry their results in the state",
      );
    }
  }
  let next: Record<string, unknown> = {};
  const body = rec.block(
    () => {
      next = fn(current);
    },
    { pure: true, suffix: ".op.body" },
  );
  const fields = (values: Record<string, unknown>) =>
    Object.fromEntries(Object.entries(values).map(([k, val]) => [k, toNode(val)]));
  const node = rec.let({
    op: "loop",
    max: options.max,
    state: fields(options.state),
    var: v,
    ...(cond ? { while: cond } : {}),
    body,
    next: fields(next),
  });
  return materialize(shape, node) as LoopState<S>;
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
  /**
   * Secondary indexes: name → indexed fields in order, or an {@link IndexDef} for unique,
   * partial, or computed indexes.
   */
  indexes?: Record<I, readonly (keyof F & string)[] | IndexDef<F>>;
  /** Rules every row must satisfy, checked on every write. A violation fails the request (422). */
  invariants?: Record<string, (row: Row<F>) => Expr<boolean>>;
  /** Rows can be inserted but never updated or deleted. */
  appendOnly?: boolean;
  /**
   * Keep a `version` field (declare it as `version: t.version()`): 1 on insert, +1 on every
   * update. Updates can then pass `ifVersion` for optimistic concurrency.
   */
  versioned?: boolean;
  /** Rows expire `after` their timestamp `field`: invisible once expired, swept in the background. */
  ttl?: { field: keyof F & string; after: Duration };
  /** Marks the entity deprecated, so a later revision may remove it. */
  deprecated?: string;
  /**
   * The parent this entity's rows belong to: `key` maps this entity's fields to the parent's
   * key fields, and must be a prefix of this entity's key (so `childrenOf` is one range scan).
   * With `onDelete: "cascade"`, deleting a parent deletes its children in the background.
   */
  parent?: ParentDef<F>;
}

/** How a child entity points at its parent. */
export interface ParentDef<F extends Fields> {
  /** The parent entity (any entity). */
  entity: { readonly name: string; readonly keyFields: readonly string[] };
  /** Child field → parent key field. */
  key: Partial<Record<keyof F & string, string>>;
  /** Delete children when the parent is deleted (in batches of 50, in the background). */
  onDelete?: "cascade";
}

/** One part of an index: a field, or a named value computed from the row. */
export type IndexField<F extends Fields> =
  | (keyof F & string)
  | { name: string; value: (row: Row<F>) => Expr<unknown> };

/** An index with options. */
export interface IndexDef<F extends Fields> {
  /** Indexed parts, in order. */
  fields: readonly IndexField<F>[];
  /** At most one row per value. A conflicting write fails with 409. Rows with a `null` part are skipped. */
  unique?: boolean;
  /** Only rows matching this predicate are indexed. */
  where?: (row: Row<F>) => Expr<boolean>;
}

/** Bounds on the field after a query's prefix. At most one lower and one upper bound. */
export interface RangeBounds {
  gt?: ExprLike<unknown>;
  gte?: ExprLike<unknown>;
  lt?: ExprLike<unknown>;
  lte?: ExprLike<unknown>;
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
  /** Bounds on the field right after the prefix, like `{ gte: from, lt: to }`. */
  range?: RangeBounds;
  /** Continue after a cursor from a previous page (`null` starts from the beginning). */
  after?: ExprLike<string | null>;
  /**
   * Values for the field right after the prefix: rows matching any of them, merged in key order
   * (at most 10 values; not with `after` or `page`). Like SQL's `IN`.
   */
  anyOf?: readonly ExprLike<unknown>[];
}

/** One page of a query: its rows and a cursor for the next page (`null` if this one wasn't full). */
export class Page<T> implements HasNode {
  readonly "~node": Node;
  readonly items: List<T>;
  readonly cursor: Expr<string | null>;

  /** @internal */
  constructor(node: Node, itemShape: Shape) {
    this["~node"] = node;
    this.items = new List<T>({ kind: "field", of: node, name: "items" }, itemShape);
    this.cursor = new Expr({ kind: "field", of: node, name: "cursor" });
  }
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
    for (const [name, def] of Object.entries<readonly string[] | IndexDef<F>>(
      this.#def.indexes ?? {},
    )) {
      const options: IndexDef<F> = isIndexDef(def)
        ? def
        : { fields: def as readonly (keyof F & string)[] };
      const self = this as unknown as Entity<F, string, string>;
      indexes[name] = {
        fields: options.fields.map((part) =>
          typeof part === "string"
            ? part
            : { name: part.name, ...rowExpression(self, `index ${name}`, part.value) },
        ),
        ...(options.unique ? { unique: true } : {}),
        ...(options.where ? { where: rowExpression(self, `index ${name}`, options.where) } : {}),
      };
    }
    const spec: EntitySpec = { key: [...this.#def.key], fields: fieldSpecs(this.fields), indexes };
    if (this.#def.versioned) spec.versioned = true;
    if (this.#def.ttl)
      spec.ttl = { field: this.#def.ttl.field, ms: durationMs(this.#def.ttl.after) };
    if (this.#def.deprecated) spec.deprecated = this.#def.deprecated;
    const invariants = Object.entries(this.#def.invariants ?? {});
    if (invariants.length > 0) {
      spec.invariants = Object.fromEntries(
        invariants.map(([name, rule]) => [name, rowExpression(this, `invariant ${name}`, rule)]),
      );
    }
    if (this.#def.appendOnly) spec.mutable = false;
    return spec;
  }

  /** @internal The key map of a row expression. */
  keyOf(row: Node): Record<string, Expr<unknown>> {
    const key: Record<string, unknown> = {};
    for (const name of this.#def.key) key[name] = new Expr({ kind: "field", of: row, name });
    return key as Record<string, Expr<unknown>>;
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

  /** Inserts a row, or updates the existing row with the same key. */
  upsert(values: InsertValues<F>): Row<F> {
    const node = recorder().let({ op: "upsert", entity: this.name, values: this.#values(values) });
    return makeRow(this, node);
  }

  /** Inserts a row unless one with the same key exists. `true` if it inserted. */
  insertIfAbsent(values: InsertValues<F>): Expr<boolean> {
    const op = { op: "insertIfAbsent" as const, entity: this.name, values: this.#values(values) };
    return new Expr(recorder().let(op));
  }

  #values(values: Partial<ValuesLike<F>>): Record<string, Node> {
    const nodes: Record<string, Node> = {};
    for (const [name, value] of Object.entries(values)) nodes[name] = toNode(value);
    return nodes;
  }

  /**
   * Updates fields of an existing row. Fails the request with 404 if it doesn't exist (or, with
   * `ifVersion`, with 409 if someone changed it since).
   */
  update(key: KeyValues<F, K>, set: Partial<ValuesLike<F>>, options: UpdateOptions = {}): Row<F> {
    const nodes: Record<string, Node> = {};
    for (const [name, value] of Object.entries(set)) nodes[name] = toNode(value);
    return makeRow(
      this,
      recorder().let({
        op: "update",
        entity: this.name,
        key: this.#key(key),
        set: nodes,
        ...(options.ifVersion === undefined ? {} : { ifVersion: toNode(options.ifVersion) }),
      }),
    );
  }

  /** @internal The declared parent, if any. */
  get parent(): ParentDef<F> | undefined {
    return this.#def.parent;
  }

  /** @internal The primary key fields. */
  get keyFields(): readonly string[] {
    return this.#def.key;
  }

  /**
   * This entity's rows belonging to a parent row (declared with `parent`), in key order: one
   * range scan over the shared key prefix.
   */
  childrenOf(parent: HasNode, options: Omit<QueryOptions<I>, "prefix" | "index">): List<Row<F>> {
    const def = this.#def.parent;
    if (!def)
      throw new AlephBuildError(`${this.name} has no parent; declare one with parent: {...}`);
    const prefix: Expr<unknown>[] = [];
    for (const field of this.#def.key) {
      const parentField = def.key[field];
      if (parentField === undefined) break;
      prefix.push(new Expr({ kind: "field", of: parent["~node"], name: parentField }));
    }
    return this.query({ ...options, prefix });
  }

  /** Deletes a row if it exists. */
  delete(key: KeyValues<F, K>): void {
    recorder().let({ op: "delete", entity: this.name, key: this.#key(key) });
  }

  /** Reads a bounded, ordered range of rows. */
  query(options: QueryOptions<I>): List<Row<F>> {
    return new List<Row<F>>(this.#scan(options, false), this.#rowShape());
  }

  /** Reads the row with these values in a (typically unique) index, or `null`: check `.exists()`. */
  findBy(index: I, values: readonly ExprLike<unknown>[]): Row<F> {
    return this.query({ index, prefix: values, limit: 1 }).first();
  }

  /** Reads one page of rows: `{ items, cursor }`. Pass the cursor back as `after` for the next. */
  page(options: QueryOptions<I>): Page<Row<F>> {
    return new Page<Row<F>>(this.#scan(options, true), this.#rowShape());
  }

  #scan(options: QueryOptions<I>, page: boolean): Node {
    return recorder().let({
      op: "query",
      entity: this.name,
      ...(options.index === undefined ? {} : { index: options.index }),
      prefix: (options.prefix ?? []).map(toNode),
      limit: toNode(options.limit),
      ...(options.order === "desc" ? { order: "desc" as const } : {}),
      ...(options.range ? { range: rangeNodes(options.range) } : {}),
      ...(options.after === undefined ? {} : { after: toNode(options.after) }),
      ...(options.anyOf === undefined ? {} : { anyOf: options.anyOf.map(toNode) }),
      ...(page ? { page: true } : {}),
    });
  }

  #rowShape(): Shape {
    return { kind: "row", entity: this as unknown as Entity<Fields, string, string> };
  }
}

function isIndexDef<F extends Fields>(def: readonly string[] | IndexDef<F>): def is IndexDef<F> {
  return !Array.isArray(def);
}

function rangeNodes(range: RangeBounds): Record<string, Node> {
  const out: Record<string, Node> = {};
  for (const bound of ["gt", "gte", "lt", "lte"] as const) {
    if (range[bound] !== undefined) out[bound] = toNode(range[bound]);
  }
  if (("gt" in out && "gte" in out) || ("lt" in out && "lte" in out)) {
    throw new AlephBuildError(
      "a range takes at most one lower bound (gt or gte) and one upper (lt or lte)",
    );
  }
  return out;
}

/** Records a pure expression over one row (a counter predicate or sum, or an invariant). */
function rowExpression<F extends Fields>(
  entity: Entity<F, string, string>,
  what: string,
  fn: (row: Row<F>) => unknown,
): { var: number; expr: Node } {
  const rec = new Recorder();
  const v = rec.newVar();
  const expr = recording(rec, () => toNode(fn(makeRow(entity, { kind: "var", id: v }))));
  if (rec.body.length > 0) {
    throw new AlephBuildError(`${what} can only use the row: it can't read or write data`);
  }
  return { var: v, expr };
}

// ─── Counters ──────────────────────────────────────────────────────────────────────────────────

/** A group part computed from the row, like the day of a timestamp. */
export interface ComputedGroup<F extends Fields, N extends string = string> {
  name: N;
  value: (row: Row<F>) => Expr<unknown>;
}

/** What an aggregate counter tracks instead of a count. */
export type CounterAggDef<F extends Fields> =
  | { min: keyof F & string }
  | { max: keyof F & string }
  | { distinct: keyof F & string };

/** How to declare a counter. */
export interface CounterDef<
  F extends Fields,
  G extends keyof F & string,
  K extends keyof F & string = keyof F & string,
  I extends string = string,
  C extends string = never,
> {
  /** The counted entity. */
  of: Entity<F, K, I>;
  /** What the count is grouped by: fields, or computed parts (`{ name, value: row => ... }`). */
  groupBy: readonly (G | ComputedGroup<F, C>)[];
  /** Only rows matching this predicate are counted. */
  where?: (row: Row<F>) => Expr<boolean>;
  /** Sum this integer over the rows instead of counting them (`null` counts as 0). */
  sum?: (row: Row<F>) => Expr<number | null>;
  /** Track a field's smallest or largest value (latest, for timestamps), or its distinct count. */
  agg?: CounterAggDef<F>;
  /** Spread a hot counter over this many shards (2-64) so concurrent writers rarely conflict. */
  shards?: number;
}

/** The value a counter reads: a count or sum, or for `min`/`max`, the field's value or `null`. */
export type CounterValue<F extends Fields, A> = A extends { min: infer M extends keyof F }
  ? FieldValue<F[M]> | null
  : A extends { max: infer M extends keyof F }
    ? FieldValue<F[M]> | null
    : number;

/** A counter: maintained on every write to its entity, read in O(1). */
export class Counter<F extends Fields, G extends string, R = number> {
  readonly name: string;
  declare readonly __result?: R;
  readonly #def: CounterDef<F, keyof F & string, keyof F & string, string, string>;

  /** @internal */
  constructor(
    name: string,
    def: CounterDef<F, keyof F & string, keyof F & string, string, string>,
  ) {
    this.name = name;
    this.#def = def;
  }

  /** @internal The manifest declaration. */
  spec(): CounterSpec {
    const of = this.#def.of as unknown as Entity<F, string, string>;
    const groupBy: IndexPart[] = this.#def.groupBy.map((part) =>
      typeof part === "string"
        ? part
        : { name: part.name, ...rowExpression(of, `counter ${this.name}`, part.value) },
    );
    const spec: CounterSpec = { of: this.#def.of.name, groupBy };
    if (this.#def.where) spec.where = rowExpression(of, `counter ${this.name}`, this.#def.where);
    if (this.#def.sum) spec.sum = rowExpression(of, `counter ${this.name}`, this.#def.sum);
    const agg = this.#def.agg;
    if (agg) {
      const [kind, field] = Object.entries(agg)[0] as ["min" | "max" | "distinct", string];
      spec.agg = { kind, field };
    }
    if (this.#def.shards !== undefined) spec.shards = this.#def.shards;
    return spec;
  }

  /** The current value for one group, by group part name. */
  get(group: { [P in G]: ExprLike<unknown> }): Expr<R> {
    const names = this.#def.groupBy.map((part) => (typeof part === "string" ? part : part.name));
    const key = names.map((name) => toNode((group as Record<string, unknown>)[name]));
    return new Expr(recorder().let({ op: "counterGet", counter: this.name, key }));
  }
}

export type { AnyField };
