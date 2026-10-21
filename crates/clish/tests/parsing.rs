mod common;

use clish::{parse, Args, ErrorKind, Value};
use common::*;

#[test]
fn parses_full_and_abbreviated() {
    let (g, m) = demo();
    let full = parse(&g, m.privileged, "configure terminal").unwrap();
    for abbr in ["conf t", "con te", "CONF TERM", "  configure   terminal  "] {
        assert_eq!(parse(&g, m.privileged, abbr).unwrap(), full, "{abbr}");
    }
}

#[test]
fn ambiguous_prefix_is_error() {
    let (g, m) = demo();
    // "show i" matches both `ip` and `interfaces`.
    let e = parse(&g, m.privileged, "show i").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Ambiguous("i".into()));
    assert_eq!(e.caret, 5);
    // But a longer prefix is fine.
    assert!(parse(&g, m.privileged, "show int").is_ok());
}

#[test]
fn incomplete_command() {
    let (g, m) = demo();
    for line in ["configure", "show ip", "show ip interface"] {
        let e = parse(&g, m.privileged, line).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Incomplete, "{line}");
        assert_eq!(e.caret, line.len());
    }
}

#[test]
fn caret_points_at_first_bad_char() {
    let (g, m) = demo();
    let e = parse(&g, m.privileged, "show ip rotue").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert_eq!(e.caret, "show ip ".len());
    // Extra token after a complete command.
    let e = parse(&g, m.privileged, "show ip route extra").unwrap_err();
    assert_eq!(e.caret, "show ip route ".len());
    // Bad parameter value.
    let e = parse(&g, m.config_if, "ip address 10.0.0.999 255.255.255.0").unwrap_err();
    assert_eq!(e.caret, "ip address ".len());
    // Non-contiguous mask is rejected at the mask token.
    let e = parse(&g, m.config_if, "ip address 10.0.0.1 255.0.255.0").unwrap_err();
    assert_eq!(e.caret, "ip address 10.0.0.1 ".len());
}

#[test]
fn typed_arguments_are_extracted() {
    let (g, m) = demo();
    let p = parse(&g, m.config, "ip route 10.0.2.0 255.255.255.0 10.0.12.2").unwrap();
    assert_eq!(p.args.ipv4("net"), Some([10, 0, 2, 0]));
    assert_eq!(p.args.mask("mask"), Some(24));
    assert_eq!(p.args.ipv4("via"), Some([10, 0, 12, 2]));
    let p = parse(&g, m.config, "hostname R1").unwrap();
    assert_eq!(
        p.args,
        Args(vec![("name".into(), Value::Word("R1".into()))])
    );
}

#[test]
fn commands_are_scoped_to_their_mode() {
    let (g, m) = demo();
    assert!(parse(&g, m.user, "configure terminal").is_err());
    assert!(parse(&g, m.config, "shutdown").is_err());
    assert!(parse(&g, m.config_if, "shutdown").is_ok());
}

#[test]
fn fallthrough_exposes_parent_mode_commands() {
    let (g, m) = demo();
    // `interface` is a config command, usable from config-if.
    assert!(parse(&g, m.config_if, "interface g0/1").is_ok());
    // But privileged commands are not, without `do`.
    assert!(parse(&g, m.config_if, "show ip route").is_err());
}

#[test]
fn masks_parse_to_prefix_lengths() {
    let (g, m) = demo();
    for (mask, len) in [
        ("255.255.255.255", 32),
        ("255.255.255.252", 30),
        ("255.0.0.0", 8),
        ("0.0.0.0", 0),
    ] {
        let p = parse(&g, m.config_if, &format!("ip address 1.2.3.4 {mask}")).unwrap();
        assert_eq!(p.args.mask("mask"), Some(len), "{mask}");
    }
}
