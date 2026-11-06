import { codeToHtml } from "shiki";

const LANGS = new Set(["ts", "tsx", "js", "json", "bash", "sh", "rust", "text", "diff", "http"]);

/** A code block, highlighted at build time (light and dark themes). */
export async function Code({ code, lang }: { code: string; lang: string }) {
  const html = await codeToHtml(code, {
    lang: LANGS.has(lang) ? lang : "text",
    themes: { light: "github-light", dark: "github-dark" },
    defaultColor: false,
  });
  // biome-ignore lint/security/noDangerouslySetInnerHtml: shiki output of our own docs sources.
  return <div className="code" dangerouslySetInnerHTML={{ __html: html }} />;
}
