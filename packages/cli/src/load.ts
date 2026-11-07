/** Loading an app: a TypeScript/JavaScript module exporting an app, or a manifest JSON file. */
import { readFile } from "node:fs/promises";
import { extname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

/** Anything with a deterministic `toJSON()`, like an `@aleph-lang/sdk` app. */
interface Buildable {
  toJSON(options?: { sourceMap?: boolean }): string;
}

function isBuildable(value: unknown): value is Buildable {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { toJSON?: unknown }).toJSON === "function"
  );
}

/**
 * Explains a failed import. The common case: a project whose package.json says
 * `"type": "commonjs"` (new npm projects do), so Node loads `app.ts` as CommonJS.
 */
export function loadFailure(path: string, error: unknown): LoadError {
  const message = error instanceof Error ? error.message : String(error);
  const commonJs =
    /Cannot use import statement outside a module|Unexpected token 'export'/.test(message) &&
    /\.[cm]?ts$/.test(path);
  return new LoadError(
    commonJs
      ? `couldn't load ${path}: Node loaded it as CommonJS, but Aleph apps are ES modules. Set "type": "module" in your package.json (npm pkg set type=module), or rename the file to app.mts.`
      : `couldn't load ${path}: ${message}`,
  );
}

/** Thrown for problems loading an app that the user can fix. */
export class LoadError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "LoadError";
  }
}

/** Imports a module (TypeScript runs through Node's type stripping). */
export async function loadModule(path: string, fresh = false): Promise<{ default?: unknown }> {
  const url = `${pathToFileURL(resolve(path)).href}${fresh ? `?v=${Date.now()}` : ""}`;
  try {
    return (await import(url)) as { default?: unknown };
  } catch (error) {
    throw loadFailure(path, error);
  }
}

/**
 * Returns the manifest JSON for `path`: the file itself if it's `.json`, otherwise the default
 * export of the module (TypeScript runs through Node's built-in type stripping).
 */
export async function loadManifest(
  path: string,
  options: { sourceMap?: boolean; fresh?: boolean } = {},
): Promise<string> {
  const file = resolve(path);
  if (extname(file) === ".json") return readFile(file, "utf8");
  let module: { default?: unknown };
  try {
    // `fresh` re-imports the app after it changed (the URL query busts Node's module cache).
    const url = `${pathToFileURL(file).href}${options.fresh ? `?v=${Date.now()}` : ""}`;
    module = (await import(url)) as { default?: unknown };
  } catch (error) {
    throw loadFailure(path, error);
  }
  if (!isBuildable(module.default)) {
    throw new LoadError(
      `${path} must \`export default\` an app created with app() from @aleph-lang/sdk`,
    );
  }
  return module.default.toJSON({ sourceMap: options.sourceMap ?? false });
}
