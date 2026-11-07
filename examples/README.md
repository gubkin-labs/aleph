# Examples

Each example is an Aleph app (`app.ts`) with a minimal React UI. You can use the app and watch
what the platform does with it.

| Example | What it shows |
|---|---|
| [`todos`](todos) | Per-user key ranges, cursor paging, a counter, a range query on an index (which skips rows with no due date, like SQL) |
| [`team`](team) | Org roles and API keys, case-insensitive unique emails, seat limits by plan, idempotent invites, a last-owner guard, an MCP tool |
| [`shop`](shop) | A queue job that calls a payment integration, retries with backoff, a dead-letter handler, an event that sends an email, a running revenue sum, a scanning cron |

Run one:

```bash
pnpm --filter @aleph-examples/shop dev
```

Vite serves the UI, and the `aleph()` Vite plugin ([`inspector`](inspector)) runs `app.ts` on the
same dev engine `aleph dev` uses, behind `/api`. Editing `app.ts` reloads the app, and its
in-memory data starts over.

The **inspector** panel on the right:
- **Call as:** switches the caller. The dev server fakes auth with headers. The platform
  authenticates real callers.
- **Activity:** every request with its route, status, and **actual usage next to its static worst
  case**. Under each request, the async work it triggered: handler runs, retries, integration
  calls, deliveries.
- **Static bounds:** the usage report `aleph check` computes before anything runs.

Each example also has tests (`pnpm --filter @aleph-examples/<name> test`) that validate the app
and run its main flows on the dev engine. `pnpm verify` runs them, typechecks the UIs, and builds them.
