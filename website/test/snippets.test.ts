/**
 * Type-checks every TypeScript snippet in the docs against the real SDK. Each snippet becomes a
 * function in a generated module that imports the prelude (the names fragments refer to), and
 * `tsc` must accept the result. A snippet whose fence says ```ts nocheck is skipped. A
 * "scope" MDX comment starts a fresh scope, for a page made of sections that stand alone.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const root = join(import.meta.dirname, "..");
const docs = join(root, "content", "docs");
const out = join(import.meta.dirname, "snippets", "generated");

function pages(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? pages(join(dir, e.name)) : e.name.endsWith(".mdx") ? [join(dir, e.name)] : [],
  );
}

/** Names the prelude exports, which fragments may use without defining. */
const PRELUDE = [
  "a",
  "Org",
  "Task",
  "Member",
  "User",
  "Project",
  "Product",
  "Order",
  "Account",
  "Post",
  "OrderPaid",
  "email",
  "mail",
  "payments",
  "inventory",
  "err",
  "t",
];
/** Free variables fragments use for values from their surrounding handler. */
const LOOSE =
  "declare const orgId: Expr<string>, orderId: Expr<string>, chargeId: Expr<string>, accountId: Expr<string>, body: any, params: any, query: any, errors: any, payload: any, ids: any[], key: any, cutoff: number, total: any, tax: any, price: any, order: any, post: any, task: any, tasks: any, list: any, lines: any, tags: any, b: any, user: any, subtotal: any, shipping: any, moreTags: any, amount: number, taskId: string, title: string, url: string, value: string, getToken: () => Promise<string>;";

/**
 * A page's snippets, in order, in one function: later snippets use what earlier ones defined,
 * as a reader would. Imports of the SDK come from the prelude; an `app.ts` import is the
 * prelude's app; other imports are kept.
 */
function moduleFor(page: string): { file: string; count: number } {
  const source = readFileSync(page, "utf8");
  const blocks = [
    ...source.matchAll(/```ts( nocheck)?\n([\s\S]*?)```|\{\/\* scope \*\/\}/g),
  ].filter((m) => !m[1]);
  const name = relative(docs, page).replace(/[/.]/g, "_");
  const imports = new Set<string>();
  let count = 0;
  const body = blocks
    .map((m, i) => {
      if (m[0].startsWith("{/*")) return `}\n\nexport async function scope${i}() {`;
      count += 1;
      const code = (m[2] ?? "")
        .split("\n")
        .map((line) => {
          const fromApp = /^import (\w+) from "\.\.?\/app(\.ts)?";/.exec(line);
          if (fromApp) return `const ${fromApp[1]} = a;`;
          if (/^import .* from "@aleph\/sdk";/.test(line)) return "";
          if (/^import /.test(line)) {
            imports.add(line);
            return "";
          }
          return /^export default /.test(line) ? "" : line;
        })
        .join("\n");
      return `// ${relative(root, page)}, snippet ${i + 1}\n${code}`;
    })
    .join("\n");
  const header = [
    `import { ${PRELUDE.join(", ")} } from "../prelude.ts";`,
    'import { $, app, verify, type Expr } from "./sdk.ts";',
    ...imports,
    LOOSE,
    `void [${PRELUDE.join(", ")}, $, app, verify];`,
  ].join("\n");
  const file = join(out, `${name}.ts`);
  writeFileSync(file, `${header}\n\nexport async function page() {\n${body}\n}\n`);
  return { file, count };
}

describe("docs snippets", () => {
  it("all type-check against the SDK", () => {
    rmSync(out, { recursive: true, force: true });
    mkdirSync(out, { recursive: true });
    writeFileSync(
      join(out, "sdk.ts"),
      'export * from "@aleph/sdk";\nexport type Routes = Record<string, { input: unknown; output: unknown; error: unknown }>;\n',
    );
    const files = pages(docs)
      .map(moduleFor)
      .filter((m) => m.count > 0);
    writeFileSync(
      join(out, "tsconfig.json"),
      JSON.stringify({
        extends: "../../../../tsconfig.base.json",
        compilerOptions: {
          noEmit: true,
          allowImportingTsExtensions: true,
          isolatedDeclarations: false,
          declaration: false,
          noUnusedLocals: false,
          // Loose context values (\`body\`, \`payload\`) are \`any\`, so callbacks over them are too.
          noImplicitAny: false,
          lib: ["es2023", "dom"],
          types: ["node"],
        },
        include: ["*.ts", "../prelude.ts"],
      }),
    );
    let output = "";
    try {
      execFileSync("npx", ["tsc", "-p", join(out, "tsconfig.json")], {
        cwd: root,
        encoding: "utf8",
      });
    } catch (error) {
      output = String((error as { stdout?: string }).stdout ?? error);
    }
    expect(files.length).toBeGreaterThan(8);
    expect(output, output).toBe("");
  }, 120_000);
});
