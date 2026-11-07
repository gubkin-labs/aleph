//! `parse_and_validate` on structure-aware mutations of the golden conformance manifests:
//! grafted subtrees, swapped names, edge-case numbers, dropped and duplicated elements. Most
//! mutants are close enough to valid to reach deep into the checker.
#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok((_, manifest)) = aleph_core_fuzz::mutated_manifest(&mut u, 8) else {
        return;
    };
    aleph_core_fuzz::validate_checked(&manifest.to_string());
});
