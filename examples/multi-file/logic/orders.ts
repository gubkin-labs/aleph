import { arg, operation, result, select } from 'aleph-language'
import { workforce } from '../auth/workforce'
import { orderLookup, orderSummary } from '../schemas/common'
import { orders } from '../storage/main'

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
      .limit(1)
      .notFound(),
  )
  .return({
    id: result('order').id,
    status: result('order').status,
    total: result('order').total,
  })
