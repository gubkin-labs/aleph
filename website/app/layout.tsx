import type { Metadata } from "next";
import Link from "next/link";
import type { ReactNode } from "react";
import { REPO } from "@/lib/nav";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "Aleph: declarative backends with known costs", template: "%s · Aleph" },
  description:
    "Aleph is an open-source declarative language for backends. Describe routes, data, and logic in TypeScript; every request has a worst-case cost that's checked before it runs.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <header className="topbar">
          <Link href="/" className="logo">
            <span className="logo-mark">ℵ</span> Aleph
          </Link>
          <nav>
            <Link href="/docs/">Docs</Link>
            <Link href="/docs/quickstart/">Quickstart</Link>
            <Link href="/docs/examples/">Examples</Link>
            <a href={REPO}>GitHub</a>
          </nav>
        </header>
        {children}
        <footer className="footer">
          <span>Aleph is open source under the Apache-2.0 license.</span>
          <span>
            <a href="/llms.txt">llms.txt</a> · <a href={REPO}>Source</a>
          </span>
        </footer>
      </body>
    </html>
  );
}
