#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::Bpdu;

fuzz_target!(|data: &[u8]| {
    if let Ok(b) = Bpdu::parse(data) {
        // What we emit parses back to the same thing.
        assert_eq!(Bpdu::parse(&b.emit()), Ok(b));
    }
});
