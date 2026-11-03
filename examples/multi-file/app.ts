import { application } from 'aleph-language'
import { workforce } from './auth/workforce'
import { scaleNumber } from './logic/calculations'
import { getOrder } from './logic/orders'
import { supportServer } from './mcp/server'
import { fooRoute } from './routes/foo'
import { getOrderRoute } from './routes/orders'
import { mainStorage } from './storage/main'

export const manifest = application('example')
  .storage(mainStorage)
  .authProfile(workforce)
  .operation(scaleNumber)
  .operation(getOrder)
  .route(fooRoute)
  .route(getOrderRoute)
  .mcp(supportServer)
  .build()
