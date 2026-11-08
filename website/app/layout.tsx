import { Analytics } from "@vercel/analytics/next";
import type { Metadata } from "next";
import { Instrument_Serif } from "next/font/google";
import Link from "next/link";
import type { ReactNode } from "react";
import { REPO } from "@/lib/nav";
import "./globals.css";

const serif = Instrument_Serif({
  weight: "400",
  style: ["normal", "italic"],
  subsets: ["latin"],
  variable: "--font-serif",
});

export const metadata: Metadata = {
  title: {
    default: "Aleph: a trusted, declarative language for server-side apps",
    template: "%s · Aleph",
  },
  description:
    "Write your routes, data, and rules in TypeScript. Aleph counts what every route can read, write, and compute, rejects what's unbounded, and runs the rest on shared infrastructure.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={serif.variable}>
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
        <Analytics />
      </body>
    </html>
  );
}
