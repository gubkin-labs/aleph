# Auth syntax exploration (draft)

The goal is Better Auth-style composition without running arbitrary tenant plugins in the shared runtime. An app selects platform-supported providers and feature plugins in one or more named auth profiles. The compiler validates plugin dependencies, generated routes/models, and configuration before deployment.

Separate two concepts:

- **Providers** establish identity: email/password, passkeys, OIDC, SAML, and similar integrations.
- **Plugins** add application features: organizations, teams, invitations, MFA, SCIM, API keys, roles, and audit events.

Plugins are declarative, versioned platform modules. A plugin may contribute routes, typed entities, policies, and lifecycle events; it cannot inject arbitrary code into the shared runtime.

## Example: B2B workforce auth

```toml
[auth.workforce]
plugins = ["organizations", "scim", "api_keys", "mfa"]
session.max_age = "12h"
session.idle_timeout = "30m"

[auth.workforce.providers.email_password]
require_verified_email = true

[auth.workforce.providers.oidc.acme]
issuer_url = "https://login.example.com"
client_id = "acme-client"
client_secret = { secret_ref = "sso.acme.client_secret" }

[auth.workforce.plugin_options.organizations]
teams = true
default_role = "member"
allow_personal_organizations = false

[auth.workforce.plugin_options.scim]
sync_users = true
sync_groups = true

[auth.workforce.roles.admin]
permissions = ["orders.read", "orders.write", "members.manage"]

[auth.workforce.roles.member]
permissions = ["orders.read"]

[auth.workforce.policies.orders_read]
requires_authentication = true
permissions = ["orders.read"]
organization_scope = "active"

[auth.workforce.models.user.fields]
employee_number = { type = "string", max_length = 32, unique_within = "organization" }
department = { type = "string", max_length = 80 }

[auth.workforce.models.session.fields]
device_label = { type = "string", max_length = 80 }

[auth.workforce.models.organization.fields]
billing_reference = { type = "string", max_length = 64, unique_within = "app_tenant" }

[auth.workforce.models.team.fields]
external_group_id = { type = "string", max_length = 128 }
```

A smaller app can use only an email/password provider and omit the organization, SCIM, and API-key plugins. Multiple profiles can support different audiences, such as customer, staff, and machine-to-machine access. Provider credentials are secret references, never literals in the manifest.

## Connect identity to routes, operations, and MCP

A gateway or MCP server selects the profile that authenticates its callers. The reusable operation owns authorization, so the same policy applies regardless of which protocol invokes it.

```toml
[[routes]]
method = "GET"
path = "/orders/:order_id"
auth = "auth.workforce"
operation = "logic.orders.get"

[logic.orders.get]
input_schema = "schemas.order_lookup"
output_schema = "schemas.order_summary"

[[logic.orders.get.steps]]
kind = "authorize"
policy = "auth.workforce.policies.orders_read"

[mcp.servers.support]
auth = "auth.workforce"
instructions = "Use tools only for the authenticated user's active organization."

[[mcp.servers.support.tools]]
name = "get_order"
description = "Read an order's status and total."
operation = "logic.orders.get"
```

The route and MCP tool reuse the operation's input/output schema and policy. MCP instructions are descriptive guidance, not enforcement. The gateway and MCP adapter establish the authenticated principal; the operation's policy checks permission and active-organization membership.

## Reference and extend built-in auth entities

Plugins provide typed system entities that logic can reference, such as:

- `auth.workforce.user`
- `auth.workforce.session`
- `auth.workforce.organization` and `auth.workforce.membership`
- `auth.workforce.team` and `auth.workforce.team_membership`

A profile operation can query the current user's public fields through the same typed query primitive used for app tables:

```toml
[logic.profile.current_user]
output_schema = "schemas.public_user"

[[logic.profile.current_user.steps]]
kind = "query"
as = "user"
table = "auth.workforce.user"
select = ["id", "email", "department"]
where = { id = "$auth.user.id" }
limit = 1

[[logic.profile.current_user.steps]]
kind = "return"
value = "$user"
```

The identity context is platform-derived; a request body cannot choose its own user, organization, or tenant. Session credentials and token material are never exposed to ordinary logic. Auth lifecycle writes—creating sessions, changing membership, revoking credentials—go through the relevant auth primitive/plugin rather than unrestricted table mutations.

`models.<entity>.fields` adds typed custom fields and lets the compiler plan schema changes. Core identity keys and plugin-owned columns stay protected. Uniqueness must name its scope (`organization` or `app_tenant`) so a shared database does not accidentally impose global uniqueness or leak cross-tenant information.

## Tenancy and enforcement boundary

Aleph's **app tenant** is the deployment/isolation boundary in the shared platform. An application's **organization** or **team** is an end-user business boundary inside that app tenant. A request can therefore carry both scopes. Storage access is always scoped to the app tenant; organization/team policies add narrower checks. Never use a client-supplied tenant or organization ID as proof of membership.

The initial plugin catalog should be curated and dependency-checked. For example, SCIM may require organizations; teams may be an option of the organizations plugin. The compiler should reject unknown plugins, incompatible combinations, undeclared model references, and attempts to alter protected fields.

## Questions for the next iteration

- Should teams be a feature of `organizations`, or a separately selectable plugin?
- Are roles and permissions enough for the first policy language, or do real use cases need bounded attribute-based conditions (for example, `record.owner_id == auth.user.id`)?
- Which auth entities should be queryable directly, and which should only expose dedicated operations? My starting point is typed read access with redaction for sessions and plugin-controlled writes for all security-sensitive fields.
