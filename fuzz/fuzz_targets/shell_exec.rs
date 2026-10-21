#![no_main]
use engine::chain::blank_chain;
use engine::shell::Shell;
use libfuzzer_sys::fuzz_target;

// Arbitrary text typed into a router or a PC shell: the parser, `?` help, Tab completion and
// every handler behind them must never panic. The first byte picks the device kind.
fuzz_target!(|data: &[u8]| {
    let Some((&kind, rest)) = data.split_first() else { return };
    let text = String::from_utf8_lossy(rest);
    let mut c = blank_chain();
    let mut sh = if kind % 2 == 0 {
        Shell::router(c.r1)
    } else {
        Shell::host(c.pc1)
    };
    if kind % 2 == 0 {
        sh.exec(&mut c.world, "enable");
        sh.exec(&mut c.world, "configure terminal");
    }
    for line in text.lines().take(64) {
        let line: String = line.chars().take(200).collect();
        let _ = sh.help(&line);
        let _ = sh.tab(&line);
        if sh.exec(&mut c.world, &line).is_none() {
            break; // the session ended
        }
    }
});
