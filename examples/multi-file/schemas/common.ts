import { schema as s } from 'aleph-language'

// Schema values are reusable descriptors, not Zod instances or callbacks.
export const orderId = s.uuid()

export const numberInput = s.object({
  number: s.number(),
})

export const numberOutput = s.object({
  number: s.number(),
})

export const orderLookup = s.object({
  orderId,
})

export const orderSummary = s.object({
  id: orderId,
  status: s.enum(['pending', 'paid', 'declined']),
  total: s.decimal({ precision: 12, scale: 2 }),
})
