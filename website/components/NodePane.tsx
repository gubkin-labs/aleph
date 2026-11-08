import { highlight } from "@/components/highlight";

/**
 * The same editor frame as a `Demo`, for a typical Node backend: its code, with the lines that go
 * wrong marked, and a ribbon across the corner. The ribbon is amber: the code was accepted, which
 * is the trouble.
 */
export function NodePane({
  title,
  file,
  start = 1,
  code,
  flag = [],
  ribbon = true,
}: {
  title: string;
  file: string;
  start?: number;
  code: string[];
  /** Line numbers (in the pane's numbering) that cause the problem. */
  flag?: number[];
  /** Show the amber "accepted" ribbon (leave it out where the tab has no verdict). */
  ribbon?: boolean;
}) {
  const width = String(start + code.length).length;
  return (
    <figure className="demo nodepane">
      <figcaption className="panel-title">
        <span>{title}</span>
      </figcaption>
      <div className="editor">
        <div className="editor-tab">{file}</div>
        {ribbon && <div className="ribbon ribbon-warn ribbon-late">accepted</div>}
        <div className="editor-body">
          {code.map((text, i) => {
            const n = start + i;
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
              <div key={i} className={flag.includes(n) ? "code-line lit lit-err" : "code-line"}>
                <span className="gutter">{String(n).padStart(width, " ")}</span>
                <span className="code-text">{highlight(text) || " "}</span>
              </div>
            );
          })}
        </div>
      </div>
    </figure>
  );
}
