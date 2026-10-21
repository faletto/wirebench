#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::ArpPacket;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = ArpPacket::parse(data) {
        assert_eq!(ArpPacket::parse(&p.emit()), Ok(p));
    }
});
