//! `parse_and_validate` on arbitrary bytes: it returns diagnostics or a valid manifest, and
//! never panics, overflows the stack, or runs long (libFuzzer's `-timeout`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        aleph_core_fuzz::validate_checked(text);
    }
});
