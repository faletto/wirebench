#![no_main]
use libfuzzer_sys::fuzz_target;

// The inspector's one-line frame description must cope with any bytes.
fuzz_target!(|data: &[u8]| {
    let s = wire::summarize(data);
    assert!(!s.is_empty());
});
