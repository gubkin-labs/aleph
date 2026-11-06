"use client";
import Link from "next/link";
import { useMemo, useState } from "react";

export interface SearchEntry {
  slug: string;
  title: string;
  description: string;
  headings: string[];
}

/** Searches page titles, descriptions, and headings, all in the browser. */
export function Search({ index }: { index: SearchEntry[] }) {
  const [query, setQuery] = useState("");
  const results = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (words.length === 0) return [];
    return index
      .map((entry) => {
        const text = [entry.title, entry.description, ...entry.headings].join(" ").toLowerCase();
        const title = entry.title.toLowerCase();
        const score = words.reduce(
          (sum, w) => sum + (text.includes(w) ? 1 : 0) + (title.includes(w) ? 2 : 0),
          0,
        );
        return { entry, score, all: words.every((w) => text.includes(w)) };
      })
      .filter((r) => r.all)
      .sort((a, b) => b.score - a.score)
      .slice(0, 8);
  }, [index, query]);
  return (
    <div className="search">
      <input
        type="search"
        placeholder="Search the docs"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Search the docs"
      />
      {results.length > 0 ? (
        <ul className="search-results">
          {results.map(({ entry }) => (
            <li key={entry.slug}>
              <Link
                href={entry.slug === "" ? "/docs/" : `/docs/${entry.slug}/`}
                onClick={() => setQuery("")}
              >
                <strong>{entry.title}</strong>
                <span>{entry.description}</span>
              </Link>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
