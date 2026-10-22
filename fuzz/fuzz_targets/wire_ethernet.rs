#![no_main]
use libfuzzer_sys::fuzz_target;
use wire::EthernetFrame;

// Parsing must never panic, and anything that parses must re-emit to something that parses
// back to the same frame.
fuzz_target!(|data: &[u8]| {
    if let Ok(f) = EthernetFrame::parse(data) {
        let again = EthernetFrame::parse(&f.emit()).expect("emitted frame re-parses");
        assert_eq!(f, again);
    }
});
