/** Command-line parsing and dispatch. */
import { writeFile } from "node:fs/promises";
import { parseArgs } from "node:util";
import { check } from "./check.js";
import { startDevServer } from "./dev.js";
import { LoadError, loadManifest } from "./load.js";

const USAGE = `aleph: build, check, and run Aleph apps

Usage:
  aleph build <app.ts> [-o manifest.json]   Write the app's manifest (stdout without -o)
  aleph check <app.ts | manifest.json>      Validate and print each route's worst-case cost
  aleph dev <app.ts> [--port 3000]          Serve the app locally on the dev engine

Dev auth headers: x-aleph-user, x-aleph-orgs (org=role,...), x-aleph-api-key,
x-aleph-api-key-scopes, idempotency-key.`;

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
    const manifest = await loadManifest(file);
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
        return result.ok ? 0 : 1;
      }
      case "dev": {
        const result = check(manifest);
        if (!result.ok) {
          io.err(result.output);
          return 1;
        }
        const port = Number(values.port ?? "3000");
        await startDevServer(manifest, { port, log: io.out });
        io.out(`aleph dev: serving ${file} on http://127.0.0.1:${port}`);
        return await new Promise<number>(() => {});
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

function parse(argv: string[]) {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      out: { type: "string", short: "o" },
      port: { type: "string", short: "p" },
      help: { type: "boolean", short: "h" },
    },
  });
  const [command, file] = positionals;
  return { command, file, values };
}
