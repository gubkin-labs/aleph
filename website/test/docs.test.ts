/** Keeps the docs consistent: every navigation entry has a page, and internal links resolve. */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { fileFor, markdownFor } from "../lib/content";
import { PAGES } from "../lib/nav";

const content = join(import.meta.dirname, "..", "content", "docs");

function allFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? allFiles(join(dir, e.name)) : [join(dir, e.name)],
  );
}

describe("docs", () => {
  it("has a page for every navigation entry, and an entry for every page", () => {
    for (const page of PAGES) {
      expect(existsSync(join(content, fileFor(page.slug))), page.slug).toBe(true);
    }
    const files = allFiles(content).map((f) => relative(content, f));
    const listed = new Set(PAGES.map((p) => fileFor(p.slug)));
    expect(files.filter((f) => !listed.has(f))).toEqual([]);
  });

  it("links only to pages that exist", () => {
    const slugs = new Set(PAGES.map((p) => (p.slug === "" ? "/docs/" : `/docs/${p.slug}/`)));
    for (const page of PAGES) {
      const source = readFileSync(join(content, fileFor(page.slug)), "utf8");
      for (const [, href] of source.matchAll(/(?:\]\(|href=")(\/docs[^)"#]*)/g)) {
        expect(slugs.has(href ?? ""), `${page.slug} links to ${href}`).toBe(true);
      }
    }
  });

  it("keeps code examples whole in the Markdown copy", () => {
    // "Copy as Markdown" and llms-full.txt drop MDX import lines, but never ones inside code.
    const quickstart = markdownFor("quickstart");
    expect(quickstart).toContain('import { app, err, t } from "@aleph-lang/sdk";');
    expect(markdownFor("")).toContain("export default a;");
  });

  it("starts every page with one title", () => {
    for (const page of PAGES) {
      const source = readFileSync(join(content, fileFor(page.slug)), "utf8");
      expect(source.startsWith("# "), page.slug).toBe(true);
      expect(source.match(/^# /gm)?.length, page.slug).toBe(1);
    }
  });
});
