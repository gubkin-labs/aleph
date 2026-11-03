# Auth SDK design sketch

This is a proposed TypeScript authoring API, not an implemented package. TypeScript builders describe auth configuration; they do not execute user callbacks in the shared runtime. The SDK emits serializable nodes that the platform validates and runs.

The profile separates **providers**, which establish identity, from **plugins**, which add platform-supported features. Providers and plugins are composed per named profile; the compiler checks their options, dependencies, routes, permissions, and model changes.

## Compose a B2B profile

```ts
import { auth, schema as s, secret } from 'aleph-language'

export const workforce = auth
  .profile('workforce')
  .provider(
    auth.providers.emailPassword({
      requireVerifiedEmail: true,
    }),
  )
  .provider(
    auth.providers.oidc('acme', {
      issuerUrl: 'https://login.example.com',
      clientId: 'acme-client',
      clientSecret: secret('sso.acme.client_secret'),
    }),
  )
  .plugin(
    auth.plugins.organizations({
      teams: true,
      defaultRole: 'member',
      allowPersonalOrganizations: false,
    }),
  )
  .plugin(auth.plugins.scim({ syncUsers: true, syncGroups: true }))
  .plugin(auth.plugins.apiKeys())
  .plugin(auth.plugins.mfa({ requiredForRoles: ['admin'] }))
  .role('admin', ['orders.read', 'orders.write', 'members.manage'])
  .role('member', ['orders.read'])
  .policy(
    'orders.read',
    auth
      .policy()
      .authenticated()
      .permission('orders.read')
      .activeOrganization(),
  )
  .extend('user', {
    employeeNumber: s.string().max(32).uniqueWithin('organization'),
    department: s.string().max(80).optional(),
  })
  .extend('session', {
    deviceLabel: s.string().max(80).optional(),
  })
  .extend('organization', {
    billingReference: s.string().max(64).uniqueWithin('appTenant'),
  })
  .extend('team', {
    externalGroupId: s.string().max(128).optional(),
  })
```

A smaller app can omit the OIDC provider and enterprise plugins. Profiles can be reused for customer, staff, and service access. Secrets are references, never literal credential values. Plugins are versioned, platform-maintained declarations—not arbitrary TypeScript plugins—and can contribute routes, models, policies, and lifecycle events.

## Use the same identity and policy through HTTP and MCP

This excerpt is intended for `examples/multi-file/app.ts`; its relative imports resolve to the modules in that directory.

```ts
import { mcp, route } from 'aleph-language'
import { getOrder } from './logic/orders'
import { workforce } from './auth/workforce'

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

The route and MCP tool bind to one operation, so the operation’s input/output schemas and authorization policy are shared. The auth profile authenticates callers; the operation policy authorizes actions. MCP instructions are guidance for clients, never an enforcement mechanism.

## Reference auth entities and extend their schemas

Profiles provide typed entity references such as:

```ts
workforce.models.user
workforce.models.session
workforce.models.organization
workforce.models.membership
workforce.models.team
workforce.models.teamMembership
```

A regular operation can query allowed fields through the same typed query builder used for app data:

```ts
import { operation, result, schema as s, select, authContext } from 'aleph-language'
import { workforce } from './auth/workforce'

const publicUser = s.object({
  id: s.uuid(),
  email: s.email(),
  department: s.string().optional(),
})

export const currentUser = operation('profile.currentUser')
  .input(s.object({}))
  .output(publicUser)
  .query(
    select('user', workforce.models.user)
      .fields({
        id: workforce.models.user.id,
        email: workforce.models.user.email,
        department: workforce.models.user.department,
      })
      .where(workforce.models.user.id.eq(authContext.user.id))
      .limit(1),
  )
  .return(result('user'))
```

The compiler generates schema changes for `.extend(...)` and protects core identity fields and plugin-owned columns. Session token material is never exposed to ordinary operations; session access is limited to safe metadata and dedicated auth operations such as revocation.

Aleph's **app tenant** is the shared-platform isolation boundary. An app's **organization** and **team** are business scopes inside that tenant. Every request carries trusted app-tenant context; organization/team policies add narrower checks. Client-provided IDs are not proof of membership.

## Decisions to resolve

- Keep roles/permissions plus active-organization scope initially, or add a restricted attribute policy builder such as `record.ownerId.eq(authContext.user.id)`?
- Should teams be an option of the organizations plugin or a separate plugin?
- Which auth entities should allow direct typed reads, and which should be accessible only through dedicated auth operations?

The guiding constraint is that every plugin and policy must compile to a known, typed capability graph. Custom JS callbacks, raw database writes to security-sensitive records, and client-controlled tenant context are not part of the runtime language.
