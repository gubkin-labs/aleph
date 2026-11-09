import { Analytics } from "@vercel/analytics/next";
import type { Metadata, Viewport } from "next";
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

const SITE = "https://aleph-lang.com";
const DESCRIPTION =
  "Every Aleph route declares what it can read, write, and compute, and anything unbounded is rejected before it ships. So an app can't flood your database or spike CPU and memory. Auth, databases, queues, and more are built in, and many apps can share one runtime.";

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#ffffff" },
    { media: "(prefers-color-scheme: dark)", color: "#0e1015" },
  ],
};

const jsonLd = {
  "@context": "https://schema.org",
  "@graph": [
    { "@type": "WebSite", name: "Aleph", url: SITE },
    {
      "@type": "SoftwareSourceCode",
      name: "Aleph",
      url: SITE,
      codeRepository: REPO,
      license: "https://www.apache.org/licenses/LICENSE-2.0",
      description: DESCRIPTION,
    },
  ],
};

export const metadata: Metadata = {
  metadataBase: new URL(SITE),
  applicationName: "Aleph",
  alternates: { canonical: "/" },
  openGraph: {
    type: "website",
    siteName: "Aleph",
    url: "/",
    locale: "en_US",
    title: "Aleph: a language for agents to write backends that scale safely",
    description: DESCRIPTION,
  },
  twitter: {
    card: "summary_large_image",
    title: "Aleph: a language for agents to write backends that scale safely",
    description: DESCRIPTION,
  },
  title: {
    default: "Aleph: a language for agents to write backends that scale safely",
    template: "%s · Aleph",
  },
  description: DESCRIPTION,
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={serif.variable}>
      <body>
        <script type="application/ld+json">{JSON.stringify(jsonLd)}</script>
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
