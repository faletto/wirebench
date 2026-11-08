#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::{Lsa, OspfPacket};

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = OspfPacket::parse(data) {
        assert_eq!(OspfPacket::parse(&p.emit()), Ok(p));
    }
    if let Ok((l, _)) = Lsa::parse(data) {
        assert_eq!(Lsa::parse(&l.emit()).map(|(x, _)| x), Ok(l));
    }
});
