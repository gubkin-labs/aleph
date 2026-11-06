/** The docs' table of contents: every page, in reading order, grouped like Diátaxis. */

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
          "A declarative language for backends whose every request has a known worst-case cost.",
      },
      {
        slug: "why-declarative",
        title: "Why declarative backends",
        description:
          "What changes when a backend is data instead of code: guarantees, tooling, and sharing.",
      },
      {
        slug: "use-cases",
        title: "Use cases",
        description: "SaaS APIs, ledgers, shops, pipelines, webhooks, and agent tools.",
      },
      {
        slug: "comparison",
        title: "Aleph vs. alternatives",
        description:
          "How Aleph compares to serverless functions, BaaS products, ORMs, and workflow engines.",
      },
    ],
  },
  {
    title: "Get started",
    pages: [
      {
        slug: "installation",
        title: "Installation",
        description: "Install the SDK and CLI.",
      },
      {
        slug: "quickstart",
        title: "Quickstart",
        description: "Write a backend, check its costs, test it, and serve it in five minutes.",
      },
      {
        slug: "tutorial",
        title: "Tutorial: a team task tracker",
        description:
          "Build a multi-user API step by step: entities, auth, queries, counters, and async work.",
      },
    ],
  },
  {
    title: "Concepts",
    pages: [
      {
        slug: "concepts/how-it-works",
        title: "How Aleph works",
        description: "From app.ts to a manifest to a validated plan run by a trusted core.",
      },
      {
        slug: "concepts/bounded-execution",
        title: "Bounded execution",
        description: "Static cost analysis, limits, and the runtime meter that enforces them.",
      },
      {
        slug: "concepts/data-model",
        title: "Data model",
        description:
          "Entities, ordered keys, indexes, counters, and why there are no unbounded scans.",
      },
      {
        slug: "concepts/transactions-and-effects",
        title: "Transactions and effects",
        description:
          "One serializable transaction per request, and effects released exactly once after commit.",
      },
      {
        slug: "concepts/types",
        title: "Types and null",
        description: "Field types, inferred response types, nullability, and narrowing.",
      },
      {
        slug: "concepts/isolation",
        title: "Tenants and isolation",
        description: "How many apps and tenants share one runtime safely.",
      },
      {
        slug: "concepts/schema-evolution",
        title: "Schema evolution",
        description: "Safe, backfilled, and breaking changes, and deprecating before removing.",
      },
    ],
  },
  {
    title: "Guides",
    pages: [
      {
        slug: "guides/routes",
        title: "Routes and HTTP",
        description:
          "Inputs, statuses, headers, cookies, redirects, errors with details, and CORS.",
      },
      {
        slug: "guides/auth",
        title: "Auth and callers",
        description:
          "Signed-in users, org roles, API keys, caller profiles, and the user directory.",
      },
      {
        slug: "guides/data",
        title: "Reading and writing data",
        description: "Get, insert, upsert, update, queries, pagination, versions, and expiry.",
      },
      {
        slug: "guides/logic",
        title: "Expressions and logic",
        description: "Guards, branches, loops, list and string helpers, decimals, and time.",
      },
      {
        slug: "guides/async",
        title: "Async work",
        description: "Events, entity triggers, queues, keyed jobs, retries, and schedules.",
      },
      {
        slug: "guides/workflows",
        title: "Durable workflows",
        description: "Multi-step processes with state, signals, timeouts, and compensation.",
      },
      {
        slug: "guides/integrations",
        title: "Integrations and webhooks",
        description: "Calling services, sending messages, and receiving signed webhooks.",
      },
      {
        slug: "guides/testing",
        title: "Testing with examples",
        description: "Worked examples on routes, run by aleph test.",
      },
      {
        slug: "guides/clients",
        title: "Typed clients and OpenAPI",
        description: "Generate a typed fetch client, OpenAPI documents, and MCP tools.",
      },
      {
        slug: "guides/local-development",
        title: "Local development",
        description: "aleph dev: watch mode, seed data, persistent data, mocks, and logs.",
      },
    ],
  },
  {
    title: "Reference",
    pages: [
      {
        slug: "reference/cli",
        title: "CLI",
        description: "Every aleph command and flag.",
      },
      {
        slug: "reference/sdk",
        title: "SDK",
        description: "The @aleph/sdk API: app, t, $, entities, lists, expressions, and workflows.",
      },
      {
        slug: "reference/limits",
        title: "Limits",
        description: "Per-request and per-job budgets, and other hard limits.",
      },
      {
        slug: "reference/manifest",
        title: "Manifest format",
        description: "The JSON format any runtime can execute, and its schema.",
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
      {
        slug: "examples",
        title: "Example apps",
        description: "Complete apps to read and run.",
      },
      {
        slug: "contributing",
        title: "Contributing",
        description: "Repository layout, local checks, and how changes land.",
      },
      {
        slug: "roadmap",
        title: "Roadmap",
        description: "What's next for the language and its tooling.",
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
