/** Command-line parsing and dispatch. */
import { readFile, writeFile } from "node:fs/promises";
import { parseArgs } from "node:util";
import { compareManifests, runExamples, validate } from "@aleph/core";
import type { Manifest } from "@aleph/manifest";
import { check, formatEvolution } from "./check.js";
import { generateClient } from "./client.js";
import { startDevServer } from "./dev.js";
import { LoadError, loadManifest, loadModule } from "./load.js";
import { mcpTools, openapi } from "./openapi.js";

const USAGE = `aleph: build, check, and run Aleph apps

Usage:
  aleph build <app.ts> [-o manifest.json] [--source-map]   Write the app's manifest
  aleph check <app.ts | manifest.json> [--against deployed.json]
                                            Validate and print worst-case usage; with --against,
                                            classify every change from the deployed manifest
  aleph test <app.ts | manifest.json>       Run every route example
  aleph client <app.ts | manifest.json> [-o client.ts]   Write a typed fetch client
  aleph dev <app.ts> [--port 3000] [--watch] [--seed seed.ts] [--data .aleph/dev.json]
            [--secret NAME=value ...]       Serve the app locally (runs async work too)
  aleph openapi <app.ts | manifest.json>    Print an OpenAPI 3.1 document for the routes
  aleph mcp <app.ts | manifest.json>        Print the MCP tools the app exposes

Dev auth headers: x-aleph-user, x-aleph-orgs (org=role,...), x-aleph-api-key,
x-aleph-api-key-scopes, x-aleph-profile (JSON), idempotency-key.`;

/** Output streams, so tests can capture them. */
export interface Io {
  out: (text: string) => void;
  err: (text: string) => void;
}

const stdio: Io = {
  out: (text) => process.stdout.write(`${text}\n`),
  err: (text) => process.stderr.write(`${text}\n`),
};

/** Runs the CLI. Returns the process exit code. */
export async function main(argv: string[], io: Io = stdio): Promise<number> {
  let parsed: ReturnType<typeof parse>;
  try {
    parsed = parse(argv);
  } catch (error) {
    io.err(`${error instanceof Error ? error.message : String(error)}\n\n${USAGE}`);
    return 2;
  }
  const { command, file, values } = parsed;
  if (values.help || !command) {
    io.out(USAGE);
    return command || values.help ? 0 : 2;
  }
  if (!file) {
    io.err(`aleph ${command} needs a file\n\n${USAGE}`);
    return 2;
  }
  try {
    // Source maps make diagnostics point at your code; build only adds them on request.
    const sourceMap = command !== "build" || Boolean(values["source-map"]);
    const manifest = await loadManifest(file, { sourceMap });
    switch (command) {
      case "build":
        if (values.out) {
          await writeFile(values.out, manifest);
          io.out(`wrote ${values.out}`);
        } else {
          io.out(manifest.trimEnd());
        }
        return 0;
      case "check": {
        const result = check(manifest);
        (result.ok ? io.out : io.err)(result.output);
        if (!result.ok) return 1;
        if (values.against) {
          const deployed = await readFile(values.against, "utf8");
          const evolution = compareManifests(deployed, manifest);
          (evolution.deployable ? io.out : io.err)(formatEvolution(evolution));
          return evolution.deployable ? 0 : 1;
        }
        return 0;
      }
      case "test": {
        const result = check(manifest);
        if (!result.ok) {
          io.err(result.output);
          return 1;
        }
        const results = runExamples(manifest);
        for (const r of results) {
          const mark = r.failures.length === 0 ? "✔" : "✖";
          io.out(`${mark} ${r.route}: ${r.name}`);
          for (const failure of r.failures) io.out(`    ${failure}`);
        }
        const failed = results.filter((r) => r.failures.length > 0).length;
        const summary = `${results.length - failed} passed, ${failed} failed`;
        (failed === 0 ? io.out : io.err)(
          `\n${results.length === 0 ? "no examples (add .example(...) to routes)" : summary}`,
        );
        return failed === 0 ? 0 : 1;
      }
      case "client": {
        const result = validate(manifest);
        if (!result.ok) {
          io.err(check(manifest).output);
          return 1;
        }
        const source = generateClient(JSON.parse(manifest) as Manifest, result.report);
        if (values.out) {
          await writeFile(values.out, source);
          io.out(`wrote ${values.out}`);
        } else {
          io.out(source.trimEnd());
        }
        return 0;
      }
      case "dev": {
        const result = check(manifest);
        if (!result.ok) {
          io.err(result.output);
          return 1;
        }
        const port = Number(values.port ?? "3000");
        const secrets = Object.fromEntries(
          (values.secret ?? []).map((pair) => {
            const at = pair.indexOf("=");
            return at < 0 ? [pair, ""] : [pair.slice(0, at), pair.slice(at + 1)];
          }),
        );
        const seed = values.seed ? await loadSeed(values.seed) : undefined;
        await startDevServer(manifest, {
          port,
          log: io.out,
          secrets,
          ...(seed ? { seed } : {}),
          ...(values.data ? { dataFile: values.data } : {}),
          ...(values.watch
            ? {
                watch: {
                  file,
                  reload: async () => {
                    const next = await loadManifest(file, { sourceMap: true, fresh: true });
                    const checked = check(next);
                    if (!checked.ok) throw new Error(checked.output);
                    return next;
                  },
                },
              }
            : {}),
        });
        io.out(
          `aleph dev: serving ${file} on http://127.0.0.1:${port}${values.watch ? " (watching)" : ""}`,
        );
        return await new Promise<number>(() => {});
      }
      case "openapi":
      case "mcp": {
        const result = check(manifest);
        if (!result.ok) {
          io.err(result.output);
          return 1;
        }
        const parsed = JSON.parse(manifest) as Manifest;
        const report = validate(manifest);
        const doc =
          command === "openapi"
            ? openapi(parsed, report.ok ? report.report : undefined)
            : { tools: mcpTools(parsed) };
        io.out(JSON.stringify(doc, null, 2));
        return 0;
      }
      default:
        io.err(`unknown command ${JSON.stringify(command)}\n\n${USAGE}`);
        return 2;
    }
  } catch (error) {
    if (error instanceof LoadError) {
      io.err(error.message);
      return 1;
    }
    throw error;
  }
}

/** Loads seed data: a JSON file or a module whose default export is `{ Entity: rows, users? }`. */
async function loadSeed(path: string): Promise<Record<string, unknown>> {
  if (path.endsWith(".json"))
    return JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
  const module = await loadModule(path);
  if (typeof module.default !== "object" || module.default === null) {
    throw new LoadError(`${path} must \`export default\` an object of rows by entity`);
  }
  return module.default as Record<string, unknown>;
}

function parse(argv: string[]) {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      out: { type: "string", short: "o" },
      port: { type: "string", short: "p" },
      against: { type: "string" },
      seed: { type: "string" },
      data: { type: "string" },
      watch: { type: "boolean", short: "w" },
      secret: { type: "string", multiple: true },
      help: { type: "boolean", short: "h" },
      "source-map": { type: "boolean" },
    },
  });
  const [command, file] = positionals;
  return { command, file, values };
}
