//! `do <command>` runs an exec command from inside a configuration mode.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::Trace;

fn router_in_interface_mode() -> (BuiltLab, Shell) {
    let mut lab = load_and_build(
        include_str!("../../../testkit/scenarios/m3_remote.lab.toml"),
        Trace::new(),
    )
    .unwrap();
    let mut sh = Shell::router(lab.devices[1].id);
    for l in ["enable", "configure terminal", "interface g0/0"] {
        sh.exec(&mut lab.world, l);
    }
    (lab, sh)
}

#[test]
fn do_runs_exec_commands_and_stays_in_the_config_mode() {
    let (mut lab, mut sh) = router_in_interface_mode();
    let out = sh.exec(&mut lab.world, "do show clock").unwrap();
    assert!(out.contains("1993"), "{out}");
    assert_eq!(sh.prompt(&lab.world), "r1(config-if)#");
    let out = sh.exec(&mut lab.world, "do show running-config").unwrap();
    assert!(out.contains("hostname r1"), "{out}");
}

#[test]
fn do_does_not_run_config_commands_and_points_at_the_bad_word() {
    let (mut lab, mut sh) = router_in_interface_mode();
    let out = sh.exec(&mut lab.world, "do hostname x").unwrap();
    assert!(out.contains("% Unrecognized input"), "{out}");
    // The caret sits under `hostname`, after the prompt and `do `.
    assert!(
        out.starts_with(&" ".repeat("r1(config-if)#do ".len())),
        "{out}"
    );
}

#[test]
fn do_has_help_and_tab_completion_for_the_command_after_it() {
    let (lab, sh) = router_in_interface_mode();
    let help = sh.help("do sh");
    assert!(help.contains("show"), "{help}");
    assert_eq!(sh.tab("do sho"), "do show ");
    assert_eq!(sh.tab("do show cl"), "do show clock ");
    // Without the space it is still the word `do` being typed.
    assert!(!sh.help("do").contains("show"));
    let _ = lab;
}
