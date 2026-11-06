#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::{Ipv4Addr, TcpSegment};

fuzz_target!(|data: &[u8]| {
    if let Ok(t) = TcpSegment::parse(data) {
        let (s, d) = (Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let raw = t.emit(s, d);
        assert_eq!(TcpSegment::parse(&raw), Ok(t));
        assert!(TcpSegment::checksum_ok(&raw, s, d));
    }
});
