/** The docs' table of contents: every page, in reading order, in four short sections. */

export interface Page {
  /** The URL path under /docs (`""` is the docs home). */
  slug: string;
  title: string;
  /** One sentence, for search, cards, and llms.txt. */
  description: string;
}

export interface Section {
  title: string;
  pages: Page[];
}

export const NAV: Section[] = [
  {
    title: "Get started",
    pages: [
      {
        slug: "",
        title: "Introduction",
        description: "What Aleph is, why a backend as a description helps, and how it runs.",
      },
      {
        slug: "quickstart",
        title: "Quickstart",
        description: "Install, describe a backend, see its worst case, test it, and serve it.",
      },
      {
        slug: "tutorial",
        title: "Tutorial: a team task tracker",
        description: "Orgs, roles, queries, counters, events, and a schedule, step by step.",
      },
      { slug: "examples", title: "Example apps", description: "Complete apps to read and run." },
    ],
  },
  {
    title: "Concepts",
    pages: [
      {
        slug: "bounded-by-construction",
        title: "Bounded by construction",
        description:
          "How every request's worst case is computed before it runs, and enforced while it runs.",
      },
      {
        slug: "shared-infrastructure",
        title: "Shared infrastructure and isolation",
        description: "Why bounded apps can share processes and databases, and how they stay apart.",
      },
      {
        slug: "running",
        title: "Running Aleph",
        description: "What a runtime needs to run Aleph apps, and the usage and logs it gets.",
      },
      {
        slug: "use-cases",
        title: "Use cases and alternatives",
        description: "What Aleph is good at, what it isn't, and how it compares.",
      },
    ],
  },
  {
    title: "Guides",
    pages: [
      {
        slug: "routes",
        title: "Routes and auth",
        description:
          "Inputs, statuses, errors, idempotency, rate limits, and who may call a route.",
      },
      {
        slug: "data",
        title: "Data",
        description: "Entities, keys, indexes, counters, reading and writing, types, and null.",
      },
      {
        slug: "logic",
        title: "Logic and expressions",
        description:
          "Guards, branches, bounded loops, list and string helpers, decimals, and time.",
      },
      {
        slug: "transactions",
        title: "Transactions and schema changes",
        description: "One transaction per request, effects released once, and safe schema changes.",
      },
      {
        slug: "background-work",
        title: "Background work and workflows",
        description: "Events, queues, schedules, and durable multi-step workflows.",
      },
      {
        slug: "integrations",
        title: "Integrations and API surface",
        description: "Outbound calls, webhooks, OpenAPI, MCP, and typed clients.",
      },
      {
        slug: "testing",
        title: "Testing and local development",
        description: "Worked examples as tests, and aleph dev.",
      },
    ],
  },
  {
    title: "Reference",
    pages: [
      { slug: "reference/cli", title: "CLI", description: "Every aleph command and flag." },
      { slug: "reference/sdk", title: "SDK", description: "The @aleph/sdk API." },
      {
        slug: "reference/limits",
        title: "Limits",
        description: "Per-request and per-job budgets, and other hard limits.",
      },
      {
        slug: "reference/diagnostics",
        title: "Diagnostics",
        description: "What each validation error means and how to fix it.",
      },
      {
        slug: "reference/manifest",
        title: "Manifest format",
        description: "The JSON format a runtime executes, and its schema.",
      },
      {
        slug: "roadmap",
        title: "Roadmap and status",
        description: "What exists today, and what's planned.",
      },
      {
        slug: "contributing",
        title: "Contributing",
        description: "Repository layout, local checks, and how changes land.",
      },
    ],
  },
];

/** Every page, in order. */
export const PAGES: Page[] = NAV.flatMap((s) => s.pages);

/** The page before and after `slug`, for footer links. */
export function neighbors(slug: string): { prev?: Page; next?: Page } {
  const i = PAGES.findIndex((p) => p.slug === slug);
  const prev = i > 0 ? PAGES[i - 1] : undefined;
  const next = i >= 0 && i < PAGES.length - 1 ? PAGES[i + 1] : undefined;
  return { ...(prev ? { prev } : {}), ...(next ? { next } : {}) };
}

/** Where the source lives, for "Edit this page" links. */
export const REPO = "https://github.com/gubkin-labs/aleph";
