#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::SyslogMessage;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = SyslogMessage::parse(data) {
        // The text may be cut at the length limit in the middle of a character, so only the
        // priority is compared.
        let back = SyslogMessage::parse(&m.emit()).unwrap();
        assert_eq!((back.facility, back.severity), (m.facility, m.severity));
    }
});
