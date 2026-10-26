#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fn walk(n: &wire::Node, len: usize) {
        assert!(n.range.start <= n.range.end && n.range.end <= len);
        n.children.iter().for_each(|c| walk(c, len));
    }
    let layers = wire::dissect(data);
    layers.iter().for_each(|n| walk(n, data.len()));
    let _ = wire::protocols(&layers);
    let _ = wire::hexdump(data);
});
