# aleph-core

The trusted core of [Aleph](https://github.com/gubkin-labs/aleph): manifest types, the validator and
static usage analyzer, and the bounded plan interpreter with its runtime usage meter.

```rust
use std::sync::Arc;
use aleph_core::{parse_and_validate, Engine, Limits, MemoryStore, SystemEnv, TenantId};

let manifest = parse_and_validate(&json, &Limits::ROUTE)?; // every diagnostic, or a ValidatedManifest
let engine = Engine::new(manifest, Arc::new(MemoryStore::new()), Arc::new(SystemEnv::new()));
let response = engine.handle(&TenantId::new("acme")?, &request);
assert!(response.usage.fits_within(&engine.manifest().report().routes[&route_id].bounds));
```

Guarantees, each tested:

- Validation returns every problem at once, with actionable messages, and never panics.
- Every execution's actual usage fits within the route's static bounds.
- Every storage key is prefixed by its tenant. Tenants can't reach each other's data.
- Every execution is one serializable optimistic transaction. Conflicts are retried, and declared
  errors roll back.
