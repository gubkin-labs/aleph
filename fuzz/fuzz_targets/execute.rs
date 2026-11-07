//! Execution: a golden manifest with a few structure-aware mutations, and if it validates, part
//! of its scenario followed by fuzzer-chosen requests, `process` calls, and clock advances on a
//! `MemoryStore` that also holds another tenant's data. After every step: usage within the
//! static bounds, no key outside the executing tenant, a failed request leaves the store
//! unchanged, and no panic.
#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok((case, manifest)) = aleph_core_fuzz::mutated_manifest(&mut u, 3) else {
        return;
    };
    let Some(valid) = aleph_core_fuzz::validate_checked(&manifest.to_string()) else {
        return;
    };
    let mut harness = aleph_core_fuzz::Harness::new(valid, case);
    let Ok(steps) = u.int_in_range(0..=harness.scenario_len()) else {
        return;
    };
    harness.replay_scenario(steps, true);
    for _ in 0..16 {
        if u.is_empty() || harness.act(&mut u).is_err() {
            break;
        }
    }
});
