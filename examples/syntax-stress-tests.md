# TypeScript SDK stress tests (draft)

These sketches test a typed TypeScript SDK as an authoring language. They are not executable examples yet. SDK calls should build a serializable plan; no callback or arbitrary TypeScript function is sent to the shared runtime. The compiler validates the generated plan independently of TypeScript's type checker.

Schemas, operations, storage, routes, MCP tools, workflows, and live handlers are reusable exported values. Importing a schema or operation reuses the same descriptor; it does not copy or reimplement its logic. Later snippets assume shared exports such as `schemas`, `tables`, `workforce`, `operations`, and `workflows` are imported from neighboring TypeScript modules.

## 1. One tenant-scoped operation, exposed through HTTP and MCP

```ts
import {
  arg,
  mcp,
  operation,
  result,
  route,
  schema as s,
  select,
} from 'aleph-language'

import { workforce } from './auth/workforce'
import { orders } from './storage/orders'

export const orderLookup = s.object({
  orderId: s.uuid(),
})

export const orderSummary = s.object({
  id: s.uuid(),
  status: s.enum(['pending', 'paid', 'declined']),
  total: s.decimal({ precision: 12, scale: 2 }),
})

export const getOrder = operation('orders.get')
  .input(orderLookup)
  .output(orderSummary)
  .authorize(workforce.policy('orders.read'))
  .query(
    select('order', orders)
      .fields({
        id: orders.id,
        status: orders.status,
        total: orders.total,
      })
      .where(orders.id.eq(arg('orderId')))
      .limit(1),
  )
  .transform('summary', {
    id: result('order').id,
    status: result('order').status,
    total: result('order').total,
  })
  .return(result('summary'))

export const getOrderRoute = route
  .get('/orders/:orderId')
  .auth(workforce)
  .operation(getOrder)

export const supportMcp = mcp
  .server('support')
  .auth(workforce)
  .instructions('Use tools only for the caller’s active organization.')
  .tool('get_order', {
    description: 'Read an order’s status and total.',
    operation: getOrder,
  })
```

The route and MCP tool use the same operation, schemas, and authorization policy. Route parameters and MCP arguments are validated against `orderLookup`. Tenant scoping comes from the trusted runtime context and table metadata, not a caller-supplied tenant ID.

## 2. Durable order processing with results passed between steps

```ts
import { arg, call, event, result, workflow, workflowContext } from 'aleph-language'

export const fulfillOrder = workflow('orders.fulfill')
  .input(schemas.orderId)
  .maxSteps(8)
  .maxDuration('48h')
  .step(
    'reservation',
    call(inventory.reserve, {
      orderId: arg('orderId'),
    }).idempotencyKey(workflowContext.id),
  )
  .step(
    'payment',
    call(payments.createIntent, {
      orderId: arg('orderId'),
      reservationId: result('reservation').id,
    }).retry({ maxAttempts: 3, backoff: 'exponential' }),
  )
  .waitFor(
    'paymentCaptured',
    event('payments.captured')
      .where(event.paymentId.eq(result('payment').id))
      .timeout('24h'),
  )
  .step(
    'shipment',
    call(shipping.createLabel, {
      orderId: arg('orderId'),
      reservationId: result('reservation').id,
      paymentId: result('payment').id,
    }),
  )
```

Each named step has a typed result available to later steps. The workflow runtime persists progress before waiting, releases compute during `waitFor`, and applies explicit retry, idempotency, step-count, and duration limits. Queue delivery retries remain distinct from workflow-step retries.

## 3. Scheduled reports with bounded per-tenant fan-out

```ts
import { schedule } from 'aleph-language'

export const dailyTenantReport = schedule('daily-tenant-report')
  .cron('0 2 * * *', { timezone: 'UTC' })
  .forEachTenant({ maxConcurrent: 10 })
  .start(workflows.reports.generate)
```

`forEachTenant` is a platform primitive, not a user-defined loop. It creates isolated, metered workflow runs with a concurrency cap. The scheduler still needs defined behavior for missed runs, overlaps, retries, and tenants added while a schedule is active.

## 4. Live AI support chat

```ts
import { arg, live, operation, result } from 'aleph-language'

export const replyToSupportMessage = operation('support.reply')
  .input(schemas.chatMessage)
  .output(schemas.chatResponse)
  .authorize(workforce.policy('support.chat.send'))
  .query(
    select('history', tables.chatMessages)
      .fields({ role: tables.chatMessages.role, content: tables.chatMessages.content })
      .where(tables.chatMessages.conversationId.eq(arg('conversationId')))
      .limit(20),
  )
  .vectorSearch('context', indexes.knowledgeBase, {
    query: arg('message'),
    topK: 5,
  })
  .model('answer', models.default)
  .prompt(prompts.supportReply)
  .context(result('history'), result('context'))
  .maxOutputTokens(600)
  .maxToolCalls(2)
  .tools(operations.getOrder)
  .return(result('answer'))

export const supportChat = live
  .room('support')
  .auth(workforce)
  .key(arg('conversationId'))
  .onMessage(replyToSupportMessage)
```

The model can invoke only declared operations, using the same tenant context and authorization. History size, retrieval count, output tokens, and tool calls are bounded. The AI step is still a metered platform primitive; the SDK does not run an arbitrary agent loop.

## 5. Checkout with parallel calls, dataflow, and finite matching

```ts
import { arg, call, match, operation, result, select } from 'aleph-language'

export const checkout = operation('checkout.quote')
  .input(schemas.checkoutInput)
  .output(schemas.checkoutResult)
  .authorize(workforce.policy('cart.checkout'))
  .query(
    select('cart', tables.carts)
      .fields({ id: tables.carts.id, subtotal: tables.carts.subtotal, items: tables.carts.items })
      .where(tables.carts.id.eq(arg('cartId')))
      .limit(1)
      .notFound(),
  )
  .parallel(
    'quotes',
    [
      call('tax', operations.taxQuote, {
        items: result('cart').items,
        postalCode: arg('postalCode'),
      }).timeout('2s'),
      call('shipping', operations.shippingQuote, {
        items: result('cart').items,
        postalCode: arg('postalCode'),
      }).timeout('2s'),
    ],
    { maxConcurrency: 2 },
  )
  .derive(
    'total',
    result('cart').subtotal
      .add(result('quotes').tax.amount)
      .add(result('quotes').shipping.amount),
  )
  .step(
    'payment',
    call(operations.authorizePayment, {
      amount: result('total'),
      method: arg('paymentMethod'),
    })
      .timeout('3s')
      .idempotencyKey(request.idempotencyKey),
  )
  .match(result('payment').status, {
    authorized: match.value({ status: 'paid', total: result('total') }),
    declined: match.value({ status: 'declined' }),
    requires_action: match.value({
      status: 'pending',
      actionUrl: result('payment').actionUrl,
    }),
    otherwise: match.fail('unexpected_payment_status'),
  })
```

`arg` and `result` create typed references, not runtime closures. `parallel` has a fixed set of calls; `match` has finite cases and returns the declared result union. A long payment callback should become a workflow wait rather than holding this request open.

## 6. Bounded collection processing

```ts
import { arg, call, item, operation, result, schema as s } from 'aleph-language'

const refreshInput = s.object({
  skuIds: s.array(s.uuid()).maxItems(100),
})

const statusList = s.array(s.enum(['available', 'unavailable'])).maxItems(100)

export const refreshInventory = operation('inventory.refresh')
  .input(refreshInput)
  .output(statusList)
  .forEach('statuses', {
    over: arg('skuIds'),
    as: 'sku',
    maxItems: 100,
    maxConcurrency: 8,
    step: call(operations.getInventoryStatus, {
      sku: item('sku'),
    }).timeout('1s'),
  })
  .return(result('statuses'))
```

The input schema also caps `skuIds` at 100. The compiler checks that the loop bound does not exceed that maximum and estimates cost as the bound times the body cost. Nested loops are initially disallowed; large batches should be chunked into durable workflows. For database work, a batch query is preferable to repeating one query per item.

## Validation and cost model to preserve

- `schema` builders produce serializable descriptors and TypeScript types; custom JavaScript refinements are not accepted.
- Binding an input/output schema adds runtime validation at the route, MCP, workflow, and operation boundaries. The server validates the emitted plan again; SDK types are not a security boundary.
- Sequential operation cost sums; `match` uses the maximum branch; bounded iteration multiplies body cost by `maxItems`; fixed parallel work sums CPU cost and separately caps concurrency.
- Async waiting uses wall time and connections even when it consumes little CPU. External calls need deadlines, call/payload budgets, and cancellation. Database `limit` bounds returned rows, not necessarily query CPU, so query shapes and statement budgets need enforcement too.
- Every operation has a runtime fuel budget and every tenant has aggregate CPU, memory, concurrency, storage, and provider quotas. Static estimates complement these runtime backstops; they do not replace them.
