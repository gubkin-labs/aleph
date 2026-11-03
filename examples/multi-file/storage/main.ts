import { field, relational, schema as s, table } from 'aleph-language'
import { orderId } from '../schemas/common'

export const posts = table('posts', {
  id: field.from(orderId).primaryKey(),
  name: field.string().max(100),
  content: field.string().optional(),
}).tenantScoped()

export const comments = table('comments', {
  id: field.uuid().primaryKey(),
  postId: field.uuid().references(posts.id),
}).tenantScoped()

export const orders = table('orders', {
  id: field.from(orderId).primaryKey(),
  organizationId: field.from(s.uuid()),
  status: field.from(s.enum(['pending', 'paid', 'declined'])),
  total: field.from(s.decimal({ precision: 12, scale: 2 })),
})
  .tenantScoped()
  .organizationScoped('organizationId')

export const mainStorage = relational('main')
  .tables(posts, comments, orders)
