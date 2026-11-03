import { route } from 'aleph-language'
import { workforce } from '../auth/workforce'
import { scaleNumber } from '../logic/calculations'

export const fooRoute = route
  .get('/foo/:number')
  .auth(workforce)
  .operation(scaleNumber)
