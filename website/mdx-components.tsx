import type { MDXComponents } from "mdx/types";
import type { ReactElement, ReactNode } from "react";
import { Code } from "./components/Code";

/** A highlighted note. */
function Note({ children, title = "Note" }: { children: ReactNode; title?: string }) {
  return (
    <aside className="callout">
      <strong>{title}</strong>
      <div>{children}</div>
    </aside>
  );
}

/** Something the maintainers still have to do by hand (publishing, hosting, ...). */
function Tbd({ children }: { children: ReactNode }) {
  return (
    <aside className="callout callout-tbd">
      <strong>TBD</strong>
      <div>{children}</div>
    </aside>
  );
}

/** A grid of linked cards. */
function Cards({ children }: { children: ReactNode }) {
  return <div className="cards">{children}</div>;
}

function Card({ href, title, children }: { href: string; title: string; children: ReactNode }) {
  return (
    <a className="card" href={href}>
      <strong>{title}</strong>
      <span>{children}</span>
    </a>
  );
}

type CodeElement = ReactElement<{ className?: string; children?: string }>;

export function useMDXComponents(components: MDXComponents): MDXComponents {
  return {
    ...components,
    pre: ({ children }) => {
      const code = children as CodeElement;
      const lang = code.props.className?.replace("language-", "") ?? "text";
      return <Code code={String(code.props.children ?? "").trimEnd()} lang={lang} />;
    },
    Note,
    Tbd,
    Cards,
    Card,
  };
}
