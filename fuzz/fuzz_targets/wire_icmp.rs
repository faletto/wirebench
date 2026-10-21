#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::{IcmpEcho, IcmpError};

fuzz_target!(|data: &[u8]| {
    if let Ok(e) = IcmpEcho::parse(data) {
        assert_eq!(IcmpEcho::parse(&e.emit()), Ok(e));
    }
    if let Ok(e) = IcmpError::parse(data) {
        assert_eq!(IcmpError::parse(&e.emit()), Ok(e));
    }
});
