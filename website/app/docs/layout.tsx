import type { ReactNode } from "react";
import { Search } from "@/components/Search";
import { Sidebar } from "@/components/Sidebar";
import { searchIndex } from "@/lib/content";
import { NAV } from "@/lib/nav";

export default function DocsLayout({ children }: { children: ReactNode }) {
  return (
    <div className="docs">
      <aside className="docs-nav">
        <Search index={searchIndex()} />
        <Sidebar nav={NAV} />
      </aside>
      <main className="docs-main">{children}</main>
    </div>
  );
}
