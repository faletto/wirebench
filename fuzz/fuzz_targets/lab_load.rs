#![no_main]
use kernel::Trace;
use libfuzzer_sys::fuzz_target;

// Any text: loading must not panic; a lab that loads must save, reload to the same content, and
// build into a world (or fail with an error) without panicking.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let Ok(loaded) = labfile::load(text) else { return };
    let saved = labfile::save(&loaded.lab);
    let again = labfile::load(&saved).expect("a saved lab loads again").lab;
    // Unknown keys may hold NaN, which is never equal to itself, so compare the known fields.
    assert_eq!(loaded.lab.devices, again.devices);
    assert_eq!(loaded.lab.links, again.links);
    assert_eq!(loaded.lab.meta, again.meta);
    let _ = engine::lab::build(&loaded.lab, Trace::new());
});
