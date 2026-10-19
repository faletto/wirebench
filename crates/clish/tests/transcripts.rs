//! Transcript tests. A `.transcript` file is a sequence of `>> input` lines, each followed by
//! the expected output and then the prompt that follows. Run with `BLESS=1` to rewrite the
//! files from actual output (then review the diff by eye: that review is the fidelity check).
//!
//! Input conventions: a trailing `?` asks for help on the text before it (not echoed into
//! the line), and a trailing `<TAB>` shows the Tab-completed line.

mod common;

use clish::{render_error, render_help, Exec, Grammar, Session, Value};
use common::*;
use std::fmt::Write;
use std::path::Path;

fn fmt_args(a: &clish::Args) -> String {
    a.0.iter()
        .map(|(n, v)| match v {
            Value::Ipv4(b) => format!("{n}={}.{}.{}.{}", b[0], b[1], b[2], b[3]),
            Value::Mask(m) => format!("{n}=/{m}"),
            Value::Int(i) => format!("{n}={i}"),
            Value::Word(w) => format!("{n}={w}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn run(g: &Grammar, m: &Modes, inputs: &[String]) -> String {
    let mut s = Session::new(m.user);
    let mut host = "r1".to_string();
    let mut out = String::new();
    writeln!(out, "{}", s.prompt(g, &host)).unwrap();
    for line in inputs {
        let prompt = s.prompt(g, &host);
        writeln!(out, ">> {line}").unwrap();
        if let Some(l) = line.strip_suffix("<TAB>") {
            writeln!(out, "{}", s.tab(g, l)).unwrap();
        } else if let Some(l) = line.strip_suffix('?') {
            let h = render_help(&s.help(g, l));
            if !h.is_empty() {
                writeln!(out, "{h}").unwrap();
            }
        } else {
            match s.exec(g, line) {
                Exec::Empty | Exec::Moved => {}
                Exec::Logout => {
                    writeln!(out, "[logout]").unwrap();
                    return out;
                }
                Exec::Error(e) => {
                    writeln!(out, "{}", render_error(&format!("{prompt}"), &e)).unwrap()
                }
                Exec::Run { handler, args } => {
                    match handler {
                        H_ENABLE | H_CONF_T => {
                            let to = if handler == H_ENABLE {
                                m.privileged
                            } else {
                                m.config
                            };
                            s.enter(to, None);
                        }
                        H_DISABLE => s.enter(m.user, None),
                        H_INTERFACE => s.enter(m.config_if, args.word("name").map(String::from)),
                        H_HOSTNAME => host = args.word("name").unwrap().to_string(),
                        _ => {}
                    }
                    let ctx = s.context().map(|c| format!(" [{c}]")).unwrap_or_default();
                    writeln!(out, "(handler {handler}: {}{ctx})", fmt_args(&args)).unwrap();
                }
            }
        }
        writeln!(out, "{}", s.prompt(g, &host)).unwrap();
    }
    out
}

fn parse_inputs(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix(">> "))
        .map(String::from)
        .collect()
}

#[test]
fn transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/transcripts");
    let bless = std::env::var("BLESS").is_ok();
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "transcript") {
            continue;
        }
        n += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let (g, m) = demo();
        let actual = run(&g, &m, &parse_inputs(&text));
        if bless {
            std::fs::write(&path, &actual).unwrap();
        } else {
            assert_eq!(
                actual,
                text,
                "transcript {} differs (BLESS=1 to update)",
                path.display()
            );
        }
    }
    assert!(n >= 3, "expected at least three transcripts, found {n}");
}
