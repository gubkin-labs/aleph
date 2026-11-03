import { mcp } from 'aleph-language'
import { workforce } from '../auth/workforce'
import { getOrder } from '../logic/orders'
import { scaleNumber } from '../logic/calculations'

export const supportServer = mcp
  .server('support')
  .auth(workforce)
  .instructions('Use tools only for the caller’s active organization.')
  .tool('scale_number', {
    description: 'Add 2 to the input number, then multiply it by 100.',
    operation: scaleNumber,
  })
  .tool('get_order', {
    description: 'Read an order’s status and total.',
    operation: getOrder,
  })
