# aleph-language

The goal of this language is to become a new way of building server side applications in a declarative, more restrictive way compared to classic server side choices like node go or python but in favor of some major advantages:

- implement best practices using the primitives of the language: for example auth with ootb scim or multi tenant
- cost reduction. the language can be deployed with other tenants on the same infra, thus reducing costs
- ootb features: for example, designing the api layer also should give you ootb MCP server interface, ootb logs, auth and so on
- no ci: this language has no tests, ci lint or build there is no need for that since its declarative and its features are restrictive

## the language's pillars

- auth
- storage: relational storage, kv storage, files storage (and probably more in the future)
- logic: Functions Workflows, Live
- gateway

## the language
Aleph apps use a declarative syntax. The compiler validates each app and emits a versioned JSON configuration for the platform to deploy.

### example (multi-file TOML)

The root manifest imports separate schema, storage, logic, and route files. Import aliases provide namespaces, so the route and calculation can both reuse `common.number`.

- [Root manifest](examples/multi-file/aleph.toml)
- [Shared schema](examples/multi-file/schemas/common.toml)
- [Storage](examples/multi-file/storage/main.toml)
- [Calculation](examples/multi-file/logic/calculations.toml)
- [Route](examples/multi-file/routes/foo.toml)

`[[imports]]` and its aliases are Aleph compiler features; TOML itself does not define imports.
