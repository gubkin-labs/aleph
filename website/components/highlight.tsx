import type { ReactNode } from "react";

// ─── Syntax colors: a tiny tokenizer, enough for SDK and Express code ───────────

const TOKEN =
  /(\/\/.*$)|("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?|`[^`]*`?)|(\b\d[\d_]*\b)|\b(import|from|const|let|export|default|return|true|false|async|await|if|try|catch)\b|(\.\w+)(?=\()|(\b[A-Z]\w*\b)/g;

export function highlight(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const cls = m[1] ? "c" : m[2] ? "s" : m[3] ? "n" : m[4] ? "k" : m[5] ? "f" : "ty";
    out.push(
      <span key={at} className={`tk-${cls}`}>
        {m[0]}
      </span>,
    );
    last = at + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
