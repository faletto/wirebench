#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::TftpPacket;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = TftpPacket::parse(data) {
        assert_eq!(TftpPacket::parse(&p.emit()), Ok(p));
    }
});
