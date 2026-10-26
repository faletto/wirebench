#![no_main]
use engine::lab::{load_and_build, LabKind, Topology};
use engine::World;
use kernel::Trace;
use libfuzzer_sys::fuzz_target;

const EMPTY: &str = "format = \"labsim-lab\"\nversion = 1\n";

fn fresh() -> (World, Topology) {
    let (w, t, _) = load_and_build(EMPTY, Trace::new()).unwrap().into_parts();
    (w, t)
}

// Arbitrary edit sequences: add, cable, move, delete a device or a cable (in place), and
// reconcile back to an earlier state (what undo does). Nothing may panic, a reconcile must
// reproduce the earlier lab exactly, and the lab that results must save, load and build again to
// the same text.
// Each op is 4 bytes: kind, a, b, c.
fuzz_target!(|data: &[u8]| {
    let (mut w, mut t) = fresh();
    let mut earlier = t.snapshot(&w);
    for op in data.chunks_exact(4).take(48) {
        let n = t.devices.len().max(1);
        let (a, b) = (usize::from(op[1]) % n, usize::from(op[2]) % n);
        match op[0] % 8 {
            0 => {
                let kind = match op[1] % 3 {
                    0 => LabKind::Host,
                    1 => LabKind::Switch,
                    _ => LabKind::Router,
                };
                let _ = t.add_device(&mut w, kind, [i32::from(op[2]) - 128, i32::from(op[3]) - 128]);
            }
            1 => {
                let _ = t.add_link(&mut w, a, b);
            }
            2 => t.move_device(a, [i32::from(op[2]), i32::from(op[3])]),
            3 => {
                let _ = t.remove_device(&mut w, a);
            }
            4 => {
                let _ = t.remove_link(&mut w, usize::from(op[1]));
            }
            5 => earlier = t.snapshot(&w),
            6 => {
                t.reconcile(&mut w, &earlier).unwrap();
                assert_eq!(labfile::save(&t.snapshot(&w)), labfile::save(&earlier));
            }
            _ => w.run_until(kernel::SimTime::from_secs(u64::from(op[1] % 4))),
        }
    }
    let text = labfile::save(&t.snapshot(&w));
    let (w2, t2, warnings) = load_and_build(&text, Trace::new()).unwrap().into_parts();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(labfile::save(&t2.snapshot(&w2)), text);
});
