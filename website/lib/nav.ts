/** The docs' table of contents: every page, in reading order, grouped by what you're building. */

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
    title: "Introduction",
    pages: [
      {
        slug: "",
        title: "What is Aleph?",
        description:
          "A declarative language for backends whose every request has a known, enforced cost.",
      },
      {
        slug: "why-declarative",
        title: "Why declarative",
        description: "What a backend described as data gives you that a program can't.",
      },
      {
        slug: "shared-infrastructure",
        title: "Shared infrastructure",
        description: "Why bounded apps can share processes and databases, and why that costs less.",
      },
      {
        slug: "bounded-by-construction",
        title: "Bounded by construction",
        description:
          "How every request's worst case is computed before it runs, and enforced while it runs.",
      },
      {
        slug: "how-it-works",
        title: "How it works",
        description: "From TypeScript to a manifest to a validated plan run by one trusted core.",
      },
      {
        slug: "use-cases",
        title: "Use cases",
        description: "What Aleph is good at, and what it isn't.",
      },
      {
        slug: "comparison",
        title: "Aleph vs. alternatives",
        description: "Serverless functions, BaaS, ORMs, and workflow engines.",
      },
    ],
  },
  {
    title: "Get started",
    pages: [
      { slug: "installation", title: "Installation", description: "Install the SDK and CLI." },
      {
        slug: "quickstart",
        title: "Quickstart",
        description: "Describe a backend, see its worst case, test it, and serve it.",
      },
      {
        slug: "tutorial",
        title: "Tutorial: a team task tracker",
        description: "Orgs, roles, queries, counters, events, and a schedule, step by step.",
      },
    ],
  },
  {
    title: "Auth",
    pages: [
      {
        slug: "auth",
        title: "Auth and callers",
        description:
          "Who may call a route: users, org roles, API keys, profiles, and the user directory.",
      },
    ],
  },
  {
    title: "Data",
    pages: [
      {
        slug: "data",
        title: "Entities, keys, and indexes",
        description: "The data model: ordered keys, declared indexes, counters, and relations.",
      },
      {
        slug: "data/reading-and-writing",
        title: "Reading and writing",
        description: "Gets, queries, pagination, writes, versions, expiry, and parents.",
      },
      {
        slug: "data/transactions",
        title: "Transactions and effects",
        description: "One transaction per request, and effects released exactly once.",
      },
      {
        slug: "data/types",
        title: "Types and null",
        description: "Field types, inferred response types, nullability, and narrowing.",
      },
      {
        slug: "data/schema-evolution",
        title: "Schema evolution",
        description: "Safe, backfilled, and breaking changes.",
      },
    ],
  },
  {
    title: "Compute",
    pages: [
      {
        slug: "compute",
        title: "Routes and HTTP",
        description:
          "Inputs, statuses, headers, cookies, redirects, errors, idempotency, rate limits, CORS.",
      },
      {
        slug: "compute/logic",
        title: "Logic and expressions",
        description:
          "Guards, branches, bounded loops, list and string helpers, decimals, and time.",
      },
      {
        slug: "compute/async",
        title: "Events, queues, and schedules",
        description: "Background work: triggers, jobs, retries, keyed jobs, cron, and batch scans.",
      },
      {
        slug: "compute/workflows",
        title: "Durable workflows",
        description: "Multi-step processes with state, signals, timeouts, and compensation.",
      },
    ],
  },
  {
    title: "Integrations",
    pages: [
      {
        slug: "integrations",
        title: "Integrations and webhooks",
        description: "Declared outbound calls and messages, and signed inbound webhooks.",
      },
    ],
  },
  {
    title: "API surface",
    pages: [
      {
        slug: "api",
        title: "OpenAPI, MCP, and typed clients",
        description: "Describe your API to people, agents, and compilers, from one declaration.",
      },
    ],
  },
  {
    title: "Verifying logic",
    pages: [
      {
        slug: "verifying",
        title: "Examples and aleph test",
        description: "Worked examples on routes: tests, documentation, and a bounds check at once.",
      },
      {
        slug: "verifying/local-development",
        title: "Local development",
        description: "aleph dev: watch mode, seed data, mocks, logs, and an inspector.",
      },
    ],
  },
  {
    title: "Running Aleph",
    pages: [
      {
        slug: "running",
        title: "Running a manifest",
        description: "What a runtime needs to run Aleph apps, and what it gets from the core.",
      },
      {
        slug: "running/isolation",
        title: "Tenants and isolation",
        description: "How unrelated apps and tenants share one runtime safely.",
      },
      {
        slug: "running/observability",
        title: "Observability",
        description: "Usage, logs, and errors you get without writing any monitoring code.",
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
        slug: "reference/manifest",
        title: "Manifest format",
        description: "The JSON format a runtime executes, and its schema.",
      },
      {
        slug: "reference/diagnostics",
        title: "Diagnostics",
        description: "What each validation error means and how to fix it.",
      },
    ],
  },
  {
    title: "Project",
    pages: [
      { slug: "examples", title: "Example apps", description: "Complete apps to read and run." },
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
