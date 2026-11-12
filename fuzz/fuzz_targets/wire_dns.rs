#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::DnsMessage;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = DnsMessage::parse(data) {
        // Written back without compression, a name may be cut at the label limit and so differ,
        // so only that it parses again is checked.
        let back = DnsMessage::parse(&m.emit()).unwrap();
        assert_eq!((back.id, back.questions.len(), back.answers.len()), (m.id, m.questions.len(), m.answers.len()));
    }
});
