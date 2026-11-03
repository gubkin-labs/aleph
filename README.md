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
the language will be written in typescript using the sdk which will provide a declarative approach.
from the sdk, configurations files will be generated to a format like json that describes how the app should behave. and from that point that platform should be able to take the config files and have the user app deployed.