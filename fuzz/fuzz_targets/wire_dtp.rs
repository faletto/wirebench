#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::Dtp;

fuzz_target!(|data: &[u8]| {
    if let Ok(d) = Dtp::parse(data) {
        assert_eq!(Dtp::parse(&d.emit()), Ok(d));
    }
});
