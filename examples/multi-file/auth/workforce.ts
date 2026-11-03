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
