/** The quickstart and tutorial code must build into valid apps whose examples pass. */
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { runExamples, validate } from "@aleph/core";
import { describe, expect, it } from "vitest";

const docs = join(import.meta.dirname, "..", "content", "docs");
const scratch = mkdtempSync(join(import.meta.dirname, ".samples-"));

/** The `ts` code blocks of a page, joined into one module. */
function program(page: string, only?: RegExp): string {
  const source = readFileSync(join(docs, page), "utf8");
  const blocks = [...source.matchAll(/```ts\n([\s\S]*?)```/g)].map((m) => m[1] ?? "");
  const code = only ? blocks.filter((b) => only.test(b)) : blocks;
  const joined = code.join("\n");
  return /export default a;/.test(joined) ? joined : `${joined}\nexport default a;\n`;
}

async function load(name: string, code: string): Promise<{ toJSON(): string }> {
  const file = join(scratch, `${name}.ts`);
  writeFileSync(file, code);
  return ((await import(pathToFileURL(file).href)) as { default: { toJSON(): string } }).default;
}

describe("docs samples", () => {
  it.each([
    ["quickstart", program("quickstart.mdx", /app\("notes"\)/)],
    ["tutorial", program("tutorial.mdx")],
  ])("the %s app is valid and its examples pass", async (name, code) => {
    const app = await load(name, code);
    const result = validate(app.toJSON());
    expect(result.ok, JSON.stringify(result)).toBe(true);
    expect(runExamples(app.toJSON()).filter((r) => r.failures.length > 0)).toEqual([]);
  });
});
