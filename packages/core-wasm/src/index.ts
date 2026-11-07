/**
 * `@aleph-lang/core`: the Rust core, compiled to WASM.
 *
 * This is the same validator, usage analyzer, and interpreter the platform runs, so anything that
 * passes here passes in production.
 */
import { createRequire } from "node:module";
import type {
  BlobInfo,
  CheckReport,
  Delivery,
  Diagnostic,
  EvolutionReport,
  ExampleResult,
  Limits,
  Manifest,
  ProcessReport,
  RecordedCall,
  Request,
  Response,
} from "@aleph-lang/manifest";

interface WasmDevEngine {
  handle(requestJson: string, nowMs: number): string;
  process(nowMs: number, maxItems: number): string;
  mock(integration: string, operation: string, responseJson: string, error?: string): void;
  takeDeliveries(): string;
  takeCalls(): string;
  setSecret(name: string, value: string): void;
  putUser(userId: string, profileJson: string): void;
  seed(entity: string, rowsJson: string, nowMs: number): number;
  snapshot(): string;
  restore(snapshotJson: string): void;
  receiveUpload(url: string, size: number, contentType: string, nowMs: number): string;
  authorizeDownload(url: string, nowMs: number): string;
  free(): void;
}

/** What a blob gateway decided: the file's metadata, or why it refused (an HTTP status). */
export type BlobResult =
  | { ok: true; info: BlobInfo }
  | { ok: false; status: number; message: string };

interface WasmBindings {
  formatVersion(): string;
  defaultLimits(): string;
  jobLimits(): string;
  validate(manifestJson: string): string;
  runExamples(manifestJson: string): string;
  compareManifests(oldJson: string, newJson: string): string;
  DevEngine: new (manifestJson: string, seed: number) => WasmDevEngine;
}

// wasm-bindgen's Node target is CommonJS and loads the .wasm file synchronously.
const wasm: WasmBindings = createRequire(import.meta.url)("../wasm/aleph_core_wasm.js");

/** The result of {@link validate}. */
export type ValidateResult =
  | { ok: true; report: CheckReport }
  | { ok: false; diagnostics: Diagnostic[] };

/** Thrown by {@link DevEngine} when the manifest is invalid. */
export class InvalidManifestError extends Error {
  readonly diagnostics: Diagnostic[];

  constructor(diagnostics: Diagnostic[]) {
    super(
      `invalid manifest:\n${diagnostics.map((d) => `  ${d.code} at ${d.path}: ${d.message}`).join("\n")}`,
    );
    this.name = "InvalidManifestError";
    this.diagnostics = diagnostics;
  }
}

/** The manifest format version this core reads. */
export function formatVersion(): string {
  return wasm.formatVersion();
}

/** The default per-route limits. */
export function defaultLimits(): Limits {
  return JSON.parse(wasm.defaultLimits()) as Limits;
}

/** The default limits for handlers: queue jobs, triggers, and cron runs. */
export function jobLimits(): Limits {
  return JSON.parse(wasm.jobLimits()) as Limits;
}

/** Validates a manifest and computes the worst-case usage of every route. */
export function validate(manifest: Manifest | string): ValidateResult {
  const json = typeof manifest === "string" ? manifest : JSON.stringify(manifest);
  return JSON.parse(wasm.validate(json)) as ValidateResult;
}

/**
 * Runs every route example (`.example(...)`) on a fresh engine with deterministic time and ids.
 * Throws {@link InvalidManifestError} if the manifest is invalid.
 */
export function runExamples(manifest: Manifest | string): ExampleResult[] {
  const json = typeof manifest === "string" ? manifest : JSON.stringify(manifest);
  try {
    return JSON.parse(wasm.runExamples(json)) as ExampleResult[];
  } catch (error) {
    throw toInvalidManifest(error);
  }
}

/** The result of {@link compareManifests}. */
export type Evolution = EvolutionReport & {
  /** No breaking change that the deployed manifest didn't deprecate first. */
  deployable: boolean;
};

/** Classifies every change from the deployed manifest to a new one, with a migration plan. */
export function compareManifests(deployed: Manifest | string, next: Manifest | string): Evolution {
  const json = (m: Manifest | string) => (typeof m === "string" ? m : JSON.stringify(m));
  return JSON.parse(wasm.compareManifests(json(deployed), json(next))) as Evolution;
}

/** Options for {@link DevEngine}. */
export interface DevEngineOptions {
  /** Seed for generated ids, so runs are reproducible. Defaults to 1. */
  seed?: number;
}

/**
 * A local engine backed by an in-memory store. Time is passed in on every request, so tests
 * control it completely.
 */
export class DevEngine {
  readonly #engine: WasmDevEngine;

  constructor(manifest: Manifest | string, options: DevEngineOptions = {}) {
    const json = typeof manifest === "string" ? manifest : JSON.stringify(manifest);
    try {
      this.#engine = new wasm.DevEngine(json, options.seed ?? 1);
    } catch (error) {
      throw toInvalidManifest(error);
    }
  }

  /** Handles one request at time `nowMs` (defaults to `Date.now()`). */
  handle(request: Request, nowMs: number = Date.now()): Response {
    return JSON.parse(this.#engine.handle(JSON.stringify(request), nowMs)) as Response;
  }

  /**
   * Runs due async work at time `nowMs`: cron ticks, handlers (events, entity triggers, queue
   * jobs), and deliveries, at most `maxItems` outbox entries.
   */
  process(nowMs: number = Date.now(), maxItems = 1000): ProcessReport {
    return JSON.parse(this.#engine.process(nowMs, maxItems)) as ProcessReport;
  }

  /** Answers calls to `integration.operation` with `response`, or fails them with `{ error }`. */
  mock(integration: string, operation: string, response: unknown | { error: string }): void {
    if (typeof response === "object" && response !== null && "error" in response) {
      this.#engine.mock(
        integration,
        operation,
        "null",
        String((response as { error: unknown }).error),
      );
    } else {
      this.#engine.mock(integration, operation, JSON.stringify(response));
    }
  }

  /** Removes and returns the integration sends and channel messages delivered so far. */
  takeDeliveries(): Delivery[] {
    return JSON.parse(this.#engine.takeDeliveries()) as Delivery[];
  }

  /** Removes and returns the integration calls made so far. */
  takeCalls(): RecordedCall[] {
    return JSON.parse(this.#engine.takeCalls()) as RecordedCall[];
  }

  /** Sets a secret, such as a webhook signing key. */
  setSecret(name: string, value: string): void {
    this.#engine.setSecret(name, value);
  }

  /** Stores a user profile in the user directory (read with `auth.User.get`). */
  putUser(userId: string, profile: Record<string, unknown>): void {
    this.#engine.putUser(userId, JSON.stringify(profile));
  }

  /**
   * Inserts seed rows into an entity, maintaining indexes and counters like a real insert.
   * Rows whose key exists are skipped. Returns how many were inserted.
   */
  seed(
    entity: string,
    rows: readonly Record<string, unknown>[],
    nowMs: number = Date.now(),
  ): number {
    return this.#engine.seed(entity, JSON.stringify(rows), nowMs);
  }

  /** Every stored entry, to persist dev data between restarts (see {@link DevEngine.restore}). */
  snapshot(): string {
    return this.#engine.snapshot();
  }

  /** Replaces the stored data with a {@link DevEngine.snapshot}. */
  restore(snapshot: string): void {
    this.#engine.restore(snapshot);
  }

  /**
   * Accepts an upload to a signed URL, as a blob gateway would: checks the URL, the content
   * type, and the size, then records the file (its bucket's upload handlers run on the next
   * `process`). Store the bytes only when this returns `ok`.
   */
  receiveUpload(
    url: string,
    size: number,
    contentType: string,
    nowMs: number = Date.now(),
  ): BlobResult {
    return JSON.parse(this.#engine.receiveUpload(url, size, contentType, nowMs)) as BlobResult;
  }

  /** Checks a signed download URL and returns the file's metadata (or why it's refused). */
  authorizeDownload(url: string, nowMs: number = Date.now()): BlobResult {
    return JSON.parse(this.#engine.authorizeDownload(url, nowMs)) as BlobResult;
  }

  /** Releases the WASM memory held by this engine. */
  dispose(): void {
    this.#engine.free();
  }
}

function toInvalidManifest(error: unknown): Error {
  if (error instanceof Error) {
    try {
      return new InvalidManifestError(JSON.parse(error.message) as Diagnostic[]);
    } catch {
      return error;
    }
  }
  return new Error(String(error));
}
