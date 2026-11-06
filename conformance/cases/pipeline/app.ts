/**
 * A document pipeline: uploads are processed asynchronously by an OCR integration, with retries
 * and a dead-letter handler, live status on a realtime channel, email notifications, an
 * append-only audit log, a page-count sum, and a daily cron that purges failed documents.
 */
import { app, err, t } from "@aleph/sdk";

const orgId = t.string().min(1).max(64);

const a = app("pipeline", { roles: ["owner", "member"] });

const Doc = a.entity("Doc", {
  key: ["orgId", "docId"],
  fields: {
    orgId,
    docId: t.id().generated(),
    title: t.string().min(1).max(120),
    status: t.enum("queued", "done", "failed").default("queued"),
    pages: t.int().max(10_000).default(0),
    summary: t.string().max(200).optional(),
    updatedAt: t.timestamp().defaultNow(),
  },
  indexes: { byStatus: ["status", "updatedAt"] },
  invariants: { pagesNotNegative: (doc) => doc.pages.gte(0) },
});

const AuditLog = a.entity("AuditLog", {
  key: ["orgId", "entryId"],
  fields: {
    orgId,
    entryId: t.id().generated(),
    message: t.string().max(300),
  },
  appendOnly: true,
});

const pagesPerOrg = a.counter("pagesPerOrg", {
  of: Doc,
  groupBy: ["orgId"],
  sum: (doc) => doc.pages,
});

const DocDone = a.event("docDone", {
  payload: { orgId, docId: t.id(), title: t.string().max(120) },
});

const Process = a.queue("process", {
  payload: { orgId, docId: t.id() },
  retries: 2,
  backoff: "1s",
  maxBackoff: "10s",
});

const Feed = a.channel("orgFeed", {
  key: { orgId },
  message: { docId: t.id(), status: t.string().max(20) },
});

const ocr = a.integration("ocr", {
  provider: "http",
  config: { baseUrl: "https://ocr.example" },
  operations: {
    extract: {
      mode: "call",
      input: { docId: t.id() },
      output: { pages: t.int().min(0).max(10_000), summary: t.string().max(2_000) },
    },
  },
});

const email = a.integration("email", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(200), subject: t.string().max(200) } },
  },
});

// ─── Routes ───────────────────────────────────────────────────────────────────────────────────

a.route("POST /orgs/:orgId/docs")
  .id("uploadDoc")
  .params({ orgId })
  .member("orgId")
  .describe("Upload a document for processing")
  .mcp("upload_doc")
  .rateLimit({ per: "user", max: 3, window: "1m" })
  .body({ title: t.string().min(1).max(120) })
  .handle((_, { params, body }) => Doc.insert({ orgId: params.orgId, title: body.title.trim() }));

a.route("GET /orgs/:orgId/docs/:docId")
  .id("getDoc")
  .params({ orgId, docId: t.id() })
  .member("orgId")
  .errors({ notFound: err.notFound("Document not found") })
  .handle((_, { params, errors }) => ({
    doc: Doc.getOrFail(params, errors.notFound),
    orgPages: pagesPerOrg.get({ orgId: params.orgId }),
  }));

a.route("POST /orgs/:orgId/docs/:docId/adjust")
  .id("adjustPages")
  .params({ orgId, docId: t.id() })
  .member("orgId", { roles: ["owner"] })
  .body({ delta: t.int().min(-10_000).max(10_000) })
  .errors({ notFound: err.notFound("Document not found") })
  .handle((_, { params, body, errors }) => {
    const doc = Doc.getOrFail(params, errors.notFound);
    return doc.update({ pages: doc.pages.plus(body.delta) });
  });

// ─── Handlers ─────────────────────────────────────────────────────────────────────────────────

a.onInsert("enqueueProcessing", Doc, (_, { payload }) => {
  Process.enqueue({ orgId: payload.after.orgId, docId: payload.after.docId });
});

a.onQueue(
  "processDoc",
  Process,
  ($, { payload, errors }) => {
    const doc = Doc.getOrFail({ orgId: payload.orgId, docId: payload.docId }, errors.missing);
    const result = ocr.call("extract", { docId: payload.docId });
    doc.update({ status: "done", pages: result.pages, summary: result.summary.truncate(200) });
    Feed.publish({ orgId: payload.orgId }, { docId: payload.docId, status: "done" });
    DocDone.emit({ orgId: payload.orgId, docId: payload.docId, title: doc.title });
    $.when(result.pages.gt(1_000), () => {
      AuditLog.insert({ orgId: payload.orgId, message: $.fmt`large document ${payload.docId}` });
    });
  },
  { errors: { missing: err.notFound("Document was deleted") } },
);

a.onDeadLetter("processFailed", Process, (_, { payload }) => {
  Doc.update({ orgId: payload.orgId, docId: payload.docId }, { status: "failed" });
  Feed.publish({ orgId: payload.orgId }, { docId: payload.docId, status: "failed" });
  AuditLog.insert({ orgId: payload.orgId, message: payload.error.truncate(300) });
});

a.onEvent("notify", DocDone, ($, { payload }) => {
  email.send("send", { to: "team@acme.dev", subject: $.fmt`Processed: ${payload.title}` });
  AuditLog.insert({ orgId: payload.orgId, message: $.fmt`processed ${payload.docId}` });
});

a.every(
  "purgeFailed",
  "1d",
  ($, { payload }) => {
    $.forEach(payload.batch, (doc) => {
      doc.delete();
    });
  },
  { scan: { entity: Doc, index: "byStatus", prefix: ["failed"], batch: 50 } },
);

export default a;
