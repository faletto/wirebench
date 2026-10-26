#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::{Ipv4Addr, UdpDatagram};

fuzz_target!(|data: &[u8]| {
    if let Ok(u) = UdpDatagram::parse(data) {
        let (s, d) = (Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let raw = u.emit(s, d);
        assert_eq!(UdpDatagram::parse(&raw), Ok(u));
        assert!(UdpDatagram::checksum_ok(&raw, s, d));
    }
});
