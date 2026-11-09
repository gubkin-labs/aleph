import type { MetadataRoute } from "next";
import { PAGES } from "@/lib/nav";

export const dynamic = "force-static";

const SITE = "https://aleph-lang.com";

/** The homepage and every docs page. */
export default function sitemap(): MetadataRoute.Sitemap {
  return [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    ...PAGES.map((p) => ({
      url: p.slug === "" ? `${SITE}/docs/` : `${SITE}/docs/${p.slug}/`,
      changeFrequency: "weekly" as const,
      priority: p.slug === "" || p.slug === "quickstart" ? 0.9 : 0.7,
    })),
  ];
}
