/** Reading the docs' MDX sources (for "Copy as Markdown", search, and llms.txt). */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { PAGES, type Page } from "./nav";

const ROOT = join(process.cwd(), "content", "docs");

/** The file a docs page lives in, relative to content/docs. */
export function fileFor(slug: string): string {
  return slug === "" ? "index.mdx" : `${slug}.mdx`;
}

/** A page's Markdown source, without MDX-only lines (imports and exports). */
export function markdownFor(slug: string): string {
  const source = readFileSync(join(ROOT, fileFor(slug)), "utf8");
  return source
    .split("\n")
    .filter((line) => !/^(import|export) /.test(line))
    .join("\n")
    .trim();
}

/** Headings of a page, for search. */
export function headingsFor(slug: string): string[] {
  return markdownFor(slug)
    .split("\n")
    .filter((line) => /^#{2,3} /.test(line))
    .map((line) => line.replace(/^#+ /, ""));
}

/** The search index: titles, descriptions, and headings of every page. */
export function searchIndex(): (Page & { headings: string[] })[] {
  return PAGES.map((page) => ({ ...page, headings: headingsFor(page.slug) }));
}
