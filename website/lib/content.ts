/** Reading the docs' MDX sources (for "Copy as Markdown", search, and llms.txt). */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { PAGES, type Page } from "./nav";

const ROOT = join(process.cwd(), "content", "docs");

/** The file a docs page lives in, relative to content/docs: `slug.mdx`, or `slug/index.mdx`. */
export function fileFor(slug: string): string {
  if (slug === "") return "index.mdx";
  return existsSync(join(ROOT, `${slug}.mdx`)) ? `${slug}.mdx` : `${slug}/index.mdx`;
}

/** A page's Markdown source, without MDX-only lines (imports and exports). */
export function markdownFor(slug: string): string {
  const source = readFileSync(join(ROOT, fileFor(slug)), "utf8");
  return source
    .split("\n")
    .filter((line) => !/^(import|export) /.test(line) && line !== "{/* scope */}")
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
