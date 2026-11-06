import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { CopyMarkdown } from "@/components/CopyMarkdown";
import { fileFor, markdownFor } from "@/lib/content";
import { neighbors, PAGES, REPO } from "@/lib/nav";

interface Props {
  params: Promise<{ slug?: string[] }>;
}

function pageFor(slug: string[] | undefined) {
  const path = (slug ?? []).join("/");
  return PAGES.find((p) => p.slug === path);
}

export function generateStaticParams() {
  return PAGES.map((p) => ({ slug: p.slug === "" ? [] : p.slug.split("/") }));
}

export const dynamicParams = false;

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const page = pageFor((await params).slug);
  return page ? { title: page.title, description: page.description } : {};
}

export default async function DocPage({ params }: Props) {
  const page = pageFor((await params).slug);
  if (!page) notFound();
  const file = fileFor(page.slug);
  const { default: Content } = (await import(`@/content/docs/${file}`)) as {
    default: () => React.JSX.Element;
  };
  const { prev, next } = neighbors(page.slug);
  const href = (slug: string) => (slug === "" ? "/docs/" : `/docs/${slug}/`);
  return (
    <article className="prose">
      <div className="page-actions">
        <CopyMarkdown markdown={markdownFor(page.slug)} />
        <a className="pill" href={`${REPO}/edit/main/website/content/docs/${file}`}>
          Edit this page
        </a>
      </div>
      <Content />
      <nav className="pager">
        {prev ? (
          <Link href={href(prev.slug)}>
            <span>Previous</span>
            {prev.title}
          </Link>
        ) : (
          <span />
        )}
        {next ? (
          <Link href={href(next.slug)} className="pager-next">
            <span>Next</span>
            {next.title}
          </Link>
        ) : null}
      </nav>
    </article>
  );
}
