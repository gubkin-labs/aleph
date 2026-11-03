import { route } from 'aleph-language'
import { workforce } from '../auth/workforce'
import { getOrder } from '../logic/orders'

export const getOrderRoute = route
  .get('/orders/:orderId')
  .auth(workforce)
  .operation(getOrder)
