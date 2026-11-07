import { PAGES } from "@/lib/nav";

export const dynamic = "force-static";

/** An index of the docs for AI assistants (https://llmstxt.org). */
export function GET(): Response {
  const lines = [
    "# Aleph",
    "",
    "> Aleph is an open-source declarative language for backends. You describe routes, data, and logic in TypeScript; the SDK compiles them into a manifest whose every request has a statically known worst-case usage, checked against fixed limits.",
    "",
    "## Docs",
    "",
    ...PAGES.map((p) => `- [${p.title}](/docs/${p.slug}${p.slug ? "/" : ""}): ${p.description}`),
    "",
    "## Full text",
    "",
    "- [All docs in one file](/llms-full.txt)",
  ];
  return new Response(`${lines.join("\n")}\n`, { headers: { "content-type": "text/plain" } });
}
