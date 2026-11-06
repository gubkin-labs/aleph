"use client";
import Link from "next/link";
import { usePathname } from "next/navigation";
import type { Section } from "@/lib/nav";

/** The docs navigation, with the current page marked. */
export function Sidebar({ nav }: { nav: Section[] }) {
  const path = usePathname().replace(/\/$/, "");
  return (
    <nav className="sidebar" aria-label="Documentation">
      {nav.map((section) => (
        <div key={section.title} className="sidebar-section">
          <div className="sidebar-title">{section.title}</div>
          <ul>
            {section.pages.map((page) => {
              const href = page.slug === "" ? "/docs" : `/docs/${page.slug}`;
              return (
                <li key={page.slug}>
                  <Link href={`${href}/`} aria-current={path === href ? "page" : undefined}>
                    {page.title}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}
