import { markdownFor } from "@/lib/content";
import { PAGES } from "@/lib/nav";

export const dynamic = "force-static";

/** Every docs page as Markdown, in reading order, for AI assistants. */
export function GET(): Response {
  const body = PAGES.map((p) => markdownFor(p.slug)).join("\n\n---\n\n");
  return new Response(`${body}\n`, { headers: { "content-type": "text/plain" } });
}
