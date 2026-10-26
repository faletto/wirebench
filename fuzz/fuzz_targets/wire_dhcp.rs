#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::DhcpMessage;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = DhcpMessage::parse(data) {
        assert_eq!(DhcpMessage::parse(&m.emit()), Ok(m));
    }
});
