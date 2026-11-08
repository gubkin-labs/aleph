/** The apps behind the homepage playground must be valid, and their examples must pass. */
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { runExamples, validate } from "@aleph-lang/core";
import { describe, expect, it } from "vitest";

const dir = join(import.meta.dirname, "..", "playground-apps");

describe("playground apps", () => {
  it.each(readdirSync(dir).filter((f) => f.endsWith(".ts")))(
    "%s is valid and its examples pass",
    async (file) => {
      const mod = (await import(pathToFileURL(join(dir, file)).href)) as {
        default: { toJSON(): string };
      };
      const manifest = mod.default.toJSON();
      const result = validate(manifest);
      expect(result.ok, JSON.stringify(result)).toBe(true);
      expect(runExamples(manifest).filter((r) => r.failures.length > 0)).toEqual([]);
    },
  );
});
