/**
 * `@aleph/core`: the Rust core, compiled to WASM.
 *
 * This is the same validator, cost analyzer, and interpreter the platform runs, so anything that
 * passes here passes in production.
 */
import { createRequire } from "node:module";
import type { CheckReport, Diagnostic, Limits, Manifest, Request, Response } from "@aleph/manifest";

interface WasmDevEngine {
  handle(requestJson: string, nowMs: number): string;
  free(): void;
}

interface WasmBindings {
  formatVersion(): string;
  defaultLimits(): string;
  validate(manifestJson: string): string;
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

/** Validates a manifest and computes the worst-case cost of every route. */
export function validate(manifest: Manifest | string): ValidateResult {
  const json = typeof manifest === "string" ? manifest : JSON.stringify(manifest);
  return JSON.parse(wasm.validate(json)) as ValidateResult;
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
