# Syntax stress tests (draft)

These are design probes for the proposed TOML-based language, not compiler-validated programs. They test whether a small set of typed operations can cover common backend tasks without allowing arbitrary tenant code to run in the shared runtime. The auth examples assume the profile and policy model in [the auth syntax draft](auth-syntax.md).

The working model is a named operation with an ordered list of typed steps. Each step is a platform-defined primitive with declared inputs, outputs, permissions, and resource limits. A route, MCP tool, queue subscription, schedule, or live session can invoke the same operation.

## 1. Tenant-scoped lookup exposed as HTTP and MCP

```toml
[schemas.order_lookup.input]
order_id = "uuid"

[schemas.order_summary]
id = "uuid"
status = "string"
total = "decimal(12,2)"

[tables.orders]
tenant_scope = "required"

[logic.orders.get]
input_schema = "schemas.order_lookup"
output_schema = "schemas.order_summary"
max_steps = 4

[[logic.orders.get.steps]]
kind = "authorize"
policy = "auth.workforce.policies.orders_read"

[[logic.orders.get.steps]]
kind = "query"
as = "order"
table = "orders"
select = ["id", "status", "total"]
limit = 1
on_empty = "not_found"

[logic.orders.get.steps.where]
id = "$input.order_id"

[[logic.orders.get.steps]]
kind = "transform"
as = "result"
fields = ["order.id", "order.status", "order.total"]

[[logic.orders.get.steps]]
kind = "return"
value = "$result"

[[routes]]
method = "GET"
path = "/orders/:order_id"
auth = "auth.workforce"
operation = "logic.orders.get"

[mcp.servers.support]
auth = "auth.workforce"
instructions = "Use these tools only for the authenticated user's active organization."

[[mcp.servers.support.tools]]
name = "get_order"
description = "Read an order's status and total."
operation = "logic.orders.get"
```

The route and MCP tool reuse the operation's input/output schemas, authorization, tenant-scoped query, and transform. The route path parameter maps to `input.order_id`; MCP supplies the same field in its tool arguments. Server instructions guide the MCP client, but are not an authorization or security control. The operation's policy is enforced for both entry points.

## 2. Durable order processing

```toml
[workflows.orders.fulfill]
input_schema = "schemas.order_id"
max_duration = "48h"
max_steps = 8

[[workflows.orders.fulfill.steps]]
kind = "call"
operation = "logic.inventory.reserve"
as = "reservation"
idempotency_key = "$workflow.id"

[[workflows.orders.fulfill.steps]]
kind = "call"
operation = "logic.payments.create_intent"
as = "payment"
retry = { max_attempts = 3, backoff = "exponential" }
idempotency_key = "$workflow.id"

[[workflows.orders.fulfill.steps]]
kind = "wait_for"
event = "payments.captured"
correlation = { payment_id = "$payment.id" }
timeout = "24h"

[[workflows.orders.fulfill.steps]]
kind = "call"
operation = "logic.shipping.create_label"
input = { reservation_id = "$reservation.id", payment_id = "$payment.id" }

[[queue_subscriptions]]
queue = "orders.created"
workflow = "workflows.orders.fulfill"
```

This probes durable checkpoints, event correlation, retries, and idempotent side effects. The runtime must persist progress and be clear about replay semantics; queue delivery retries and workflow-step retries remain separate policies. Open design questions include compensation when payment or shipping fails and how finite conditional branches are represented.

## 3. Scheduled per-tenant reports

```toml
[[schedules]]
name = "daily-tenant-report"
cron = "0 2 * * *"
timezone = "UTC"
workflow = "workflows.reports.generate"
tenant_scope = "per_tenant"
max_concurrent_runs = 10
```

This probes tenant fan-out and scheduling semantics: how to cap concurrent runs, handle missed schedules, and avoid duplicate reports after a retry. The schedule should start bounded workflow instances; it should not create an unmetered loop over every tenant.

## 4. Live AI support chat

```toml
[live.support_chat]
connection_policy = "support.member"
room_key = "input.conversation_id"
on_message = "logic.support.reply"

[indexes.knowledge_base]
tenant_scope = "required"

[logic.support.reply]
input_schema = "schemas.chat_message"
output_schema = "schemas.chat_response"
max_steps = 8

[[logic.support.reply.steps]]
kind = "authorize"
policy = "support.chat.send"

[[logic.support.reply.steps]]
kind = "query"
as = "history"
table = "chat_messages"
select = ["role", "content"]
limit = 20

[logic.support.reply.steps.where]
conversation_id = "$input.conversation_id"

[[logic.support.reply.steps]]
kind = "vector_search"
as = "context"
index = "knowledge_base"
query = "$input.message"
top_k = 5

[[logic.support.reply.steps]]
kind = "model"
as = "answer"
provider = "default"
prompt = "prompts.support_reply"
context = ["$history", "$context"]
max_output_tokens = 600
max_tool_calls = 2
tools = ["logic.orders.get"]
```

This probes WebSocket/session authorization, bounded history and retrieval, model budgets, and calls from an AI model into ordinary app operations. The search step inherits tenant filtering from the index declaration. Tool calls must use the same tenant context and authorization as HTTP/MCP calls; the model cannot invent capabilities or run an unbounded agent loop.

## 5. Checkout: pass results, parallel async calls, and match

```toml
[schemas.checkout_input]
cart_id = "uuid"
postal_code = "string"
payment_method = "string"

[logic.checkout.quote]
input_schema = "schemas.checkout_input"
output_schema = "schemas.checkout_result"
max_steps = 12

[[logic.checkout.quote.steps]]
kind = "authorize"
policy = "cart.checkout"

[[logic.checkout.quote.steps]]
kind = "query"
as = "cart"
table = "carts"
select = ["id", "subtotal", "items"]
where = { id = "$input.cart_id" }
limit = 1
on_empty = "not_found"

[[logic.checkout.quote.steps]]
kind = "parallel"
as = "quotes"
max_concurrency = 2

[[logic.checkout.quote.steps.calls]]
as = "tax"
operation = "logic.tax.quote"
timeout = "2s"
input.items = "$cart.items"
input.postal_code = "$input.postal_code"

[[logic.checkout.quote.steps.calls]]
as = "shipping"
operation = "logic.shipping.quote"
timeout = "2s"
input.items = "$cart.items"
input.postal_code = "$input.postal_code"

[[logic.checkout.quote.steps]]
kind = "derive"
as = "total"
expression = "$cart.subtotal + $quotes.tax.amount + $quotes.shipping.amount"

[[logic.checkout.quote.steps]]
kind = "call"
as = "payment"
operation = "logic.payments.authorize"
timeout = "3s"
idempotency_key = "$request.idempotency_key"
input.amount = "$total"
input.method = "$input.payment_method"

[[logic.checkout.quote.steps]]
kind = "match"
as = "result"
on = "$payment.status"
cases = [
  { value = "authorized", result = { status = "paid", total = "$total" } },
  { value = "declined", result = { status = "declined" } },
  { value = "requires_action", result = { status = "pending", action_url = "$payment.action_url" } },
]
default = { error = "unexpected_payment_status" }

[[logic.checkout.quote.steps]]
kind = "return"
value = "$result"
```

The `as` names make dataflow explicit: tax and shipping receive the cart, their results become `$quotes.tax` and `$quotes.shipping`, and the payment call receives `$total`. `parallel` contains two statically declared calls; each has a deadline and the fan-out is capped at two. `match` has a finite set of cases and a default, and every result must conform to the operation's declared output union.

Waiting on tax, shipping, or payment I/O consumes wall time and connection capacity, not interpreter CPU. CPU is still charged to the operation's fuel budget. If payment requires a callback hours later, use a workflow `wait_for` step so the function invocation can finish and release compute.

## 6. Bounded batch operation

```toml
[schemas.refresh_input]
sku_ids = { type = "array", items = "uuid", max_items = 100 }

[logic.inventory.refresh]
input_schema = "schemas.refresh_input"
output_schema = "schemas.status_list"

[[logic.inventory.refresh.steps]]
kind = "for_each"
over = "$input.sku_ids"
item_as = "sku"
results_as = "statuses"
max_items = 100
max_concurrency = 8
operation = "logic.inventory.get_status"
timeout = "1s"
input.sku = "$sku"

[[logic.inventory.refresh.steps]]
kind = "return"
value = "$statuses"
```

The compiler can require `max_items` to be a literal no larger than the input schema's bound, calculate a worst-case operation budget as `100 × cost(logic.inventory.get_status)`, and cap fan-out at eight. Runtime rejects an oversized input rather than silently processing it. Nested loops should be rejected initially; large batches should use chunked durable workflows. If each item would issue the same database query, the compiler should flag the N+1 pattern and prefer a bounded batch-query primitive.

## What these examples reveal

- **Typed dataflow:** every step output has a declared type and name (`as = "cart"`; later use `$cart.items`). Reject unknown names, type mismatches, and accidental secret-to-public output flows.
- **Cost model:** give each primitive a maximum fuel cost. Sequential cost sums; `match` cost is the maximum branch; `for_each` cost is `max_items × body cost`; fixed `parallel` cost sums child costs while its concurrency limit bounds simultaneous resource use.
- **Bounded iteration:** no general `while` loops or recursion. `for_each` requires a literal `max_items` no larger than the input schema's maximum; cap nesting, output size, and per-tenant fan-out. Use chunked workflows for larger jobs.
- **Finite branching:** `match` checks an enum/tag, has exhaustive cases or an explicit default, and returns a declared union type. Side-effecting branches need their own bounded step lists and failure semantics.
- **Async semantics:** request-time calls require deadlines, response-size and call-count limits, and bounded concurrency. Waiting uses wall time and connections even when it consumes little CPU. Durable waits belong in workflows and must have timeouts or cancellation policy. Parallel calls should join; define whether one failure cancels siblings. Detached work should use a queue or workflow, not an untracked promise.
- **Safe queries:** a row limit bounds returned data, not necessarily database CPU. Use a typed query AST, automatic tenant scope, allowed/indexed query shapes, database statement timeouts, and query budgets; avoid raw SQL in the shared runtime.
- **Side effects:** mutations and external calls need explicit idempotency, retry, timeout, and transaction semantics.
- **Runtime backstop:** static estimates are not a hard sandbox. Enforce fuel, CPU/memory quotas, cancellation, and per-tenant concurrency at runtime as well.
- **MCP surface:** tools reference operations; server instructions are descriptive. Prompts and resources can be separate declarative capabilities, but instructions never substitute for authorization.
- **Resource accounting:** meter operations, workflows, tenant queues, live connections, database work, and AI-provider usage independently.
