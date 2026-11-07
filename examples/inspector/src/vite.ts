/**
 * A Vite plugin that runs an Aleph app on the dev engine next to the UI, and proxies `/api/*`
 * to it. The app reloads when its file changes (its in-memory data starts over).
 */
import type { Server } from "node:http";
import { resolve } from "node:path";
import { check, loadManifest, type MockResponse, startDevServer } from "@aleph-lang/cli";
import type { Plugin } from "vite";

/** Options for {@link aleph}. */
export interface AlephPluginOptions {
  /** The app module, relative to the Vite root, for example `"./app.ts"`. */
  app: string;
  /** Port for the Aleph dev server. */
  port: number;
  /** Integration responses to start with. */
  mocks?: MockResponse[];
}

export function aleph(options: AlephPluginOptions): Plugin {
  let server: Server | undefined;
  let appPath = options.app;

  async function start(fresh: boolean): Promise<void> {
    const manifest = await loadManifest(appPath, { sourceMap: true, fresh });
    const result = check(manifest);
    if (!result.ok) {
      console.error(`\n[aleph] ${options.app} is invalid:\n${result.output}\n`);
      return;
    }
    await new Promise<void>((done) => (server ? server.close(() => done()) : done()));
    server = await startDevServer(manifest, {
      port: options.port,
      ...(options.mocks ? { mocks: options.mocks } : {}),
      log: (line) => console.log(`[aleph] ${line}`),
    });
    console.log(`[aleph] serving ${options.app} on http://127.0.0.1:${options.port}`);
  }

  const target = `http://127.0.0.1:${options.port}`;
  return {
    name: "aleph",
    config: () => ({
      server: {
        proxy: {
          "/api": { target, rewrite: (path: string) => path.replace(/^\/api/, "") },
          "/__aleph": { target },
        },
      },
    }),
    configResolved(config) {
      appPath = resolve(config.root, options.app);
    },
    async configureServer(dev) {
      await start(false);
      dev.watcher.add(appPath);
      dev.watcher.on("change", (file) => {
        if (file === appPath) {
          console.log("[aleph] app changed, restarting (data resets)");
          start(true).catch((error: unknown) => console.error("[aleph]", error));
        }
      });
      dev.httpServer?.on("close", () => server?.close());
    },
  };
}
