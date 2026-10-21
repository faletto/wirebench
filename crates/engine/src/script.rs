//! Scenario scripts: a topology plus per-device command lists, run through the real shells.
//!
//! ```text
//! topology blank_chain
//! @r1
//! enable
//! configure terminal
//! ```
//!
//! `@name` selects the terminal that subsequent lines are typed into. `#` starts a comment.
//! A trailing `?` shows help instead of executing. The output is a transcript, ending with
//! the trace hash, so a script doubles as a golden test.

use crate::chain::blank_chain;
use crate::shell::Shell;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn run_script(text: &str) -> Result<String, String> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    match lines.next() {
        Some("topology blank_chain") => {}
        other => return Err(format!("expected `topology blank_chain`, got {other:?}")),
    }
    let mut chain = blank_chain();
    let ids: BTreeMap<&str, (kernel::DeviceId, bool)> = [
        ("pc1", (chain.pc1, false)),
        ("r1", (chain.r1, true)),
        ("r2", (chain.r2, true)),
        ("pc2", (chain.pc2, false)),
    ]
    .into_iter()
    .collect();
    let mut shells: BTreeMap<&str, Shell> = BTreeMap::new();
    let mut out = String::new();
    let mut current: Option<&str> = None;
    for line in lines {
        if let Some(name) = line.strip_prefix('@') {
            let (key, _) = ids
                .get_key_value(name)
                .ok_or(format!("unknown device {name}"))?;
            current = Some(key);
            let (id, is_router) = ids[name];
            shells.entry(key).or_insert_with(|| {
                if is_router {
                    Shell::router(id)
                } else {
                    Shell::host(id)
                }
            });
            writeln!(out, "--- {name}").unwrap();
            continue;
        }
        let name = current.ok_or("command before any @device")?;
        let sh = shells.get_mut(name).unwrap();
        let prompt = sh.prompt(&chain.world);
        writeln!(out, "{prompt}{line}").unwrap();
        let text = match line.strip_suffix('?') {
            Some(l) => sh.help(l),
            None => match sh.exec(&mut chain.world, line) {
                Some(t) => t,
                None => "[session closed]".to_string(),
            },
        };
        if !text.is_empty() {
            writeln!(out, "{text}").unwrap();
        }
    }
    writeln!(out, "--- trace {:016x}", chain.world.trace_hash()).unwrap();
    Ok(out)
}
