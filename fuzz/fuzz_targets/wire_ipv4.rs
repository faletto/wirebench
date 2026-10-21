#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::Ipv4Packet;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = Ipv4Packet::parse(data) {
        assert_eq!(Ipv4Packet::parse(&p.emit()), Ok(p));
    }
});
