import { defineConfig } from "vitest/config";

/** Resolve workspace packages to their TypeScript sources, so tests never need a build. */
const conditions = ["@aleph/source", "node", "import", "module", "default"];

export default defineConfig({
  resolve: { conditions },
  ssr: { resolve: { conditions, externalConditions: conditions } },
  test: {
    include: ["test/**/*.test.ts"],
  },
});
