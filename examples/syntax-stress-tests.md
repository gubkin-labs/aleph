# Syntax stress tests (draft)

These are design probes for the proposed TOML-based language, not compiler-validated programs. They test whether a small set of typed operations can cover common backend tasks without allowing arbitrary tenant code to run in the shared runtime.

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
policy = "orders.read"

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
operation = "logic.orders.get"

[mcp.servers.support]
instructions = "Use these tools only for the authenticated user's tenant."

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

## What these examples reveal

- **Bounded iteration:** avoid general loops and recursion. If real apps need collection processing, add a dedicated `for_each` primitive with a statically checked `max_items`, or use chunked workflows.
- **Finite branching:** workflows and operations need a constrained `when`/`choose` primitive with statically visible branches.
- **Typed step results:** `as` names and `$input` references need compile-time type checking; implicit “current value” behavior may be convenient but harder to debug.
- **Safe queries:** query and mutation steps should use a typed query AST, tenant-scope tables automatically, cap result sizes, and avoid raw SQL in the shared runtime.
- **Side effects:** mutations and external calls need explicit idempotency, retry, timeout, and transaction semantics.
- **MCP surface:** start with server instructions and tools referencing operations. Prompts and resources can be added as separate declarative capabilities; instructions must never substitute for authorization.
- **Resource accounting:** enforce budgets at operation, workflow, tenant, connection, and AI-provider boundaries—not just with a maximum step count.
