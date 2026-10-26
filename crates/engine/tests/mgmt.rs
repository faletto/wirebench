//! Management plane: enable secret and password, console login, banner, saving, reload.

use engine::chain::blank_chain;
use engine::lab::load_and_build;
use engine::shell::{running_config, switch_config, Shell, PASSWORD_PROMPT};
use engine::World;
use kernel::Trace;
use netdev::{L2Switch, Router};

const ONE_ROUTER: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "r1"
template = "router-2g"
pos = [0, 0]

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [100, 0]
"#;

/// Run lines and return each one's output.
fn run(sh: &mut Shell, w: &mut World, lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| sh.exec(w, l).unwrap()).collect()
}

fn router() -> (World, Shell) {
    let c = blank_chain();
    let sh = Shell::router(c.r1);
    (c.world, sh)
}

/// Configure from wherever the session is: `enable` first only if it is still unprivileged.
fn conf(sh: &mut Shell, w: &mut World, lines: &[&str]) {
    if sh.prompt(w).ends_with('>') {
        sh.exec(w, "enable").unwrap();
    }
    sh.exec(w, "configure terminal").unwrap();
    for l in lines {
        let out = sh.exec(w, l).unwrap();
        assert!(out.is_empty(), "`{l}` printed {out:?}");
    }
    sh.exec(w, "end").unwrap();
}

fn cfg(sh: &mut Shell, w: &mut World) -> String {
    run(sh, w, &["show running-config"]).remove(0)
}

#[test]
fn enable_without_a_password_goes_straight_to_privileged() {
    let (mut w, mut sh) = router();
    run(&mut sh, &mut w, &["enable"]);
    assert_eq!(sh.prompt(&w), "r1#");
}

#[test]
fn enable_secret_asks_for_the_password_and_hides_it() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["enable secret cisco123"]);
    let text = cfg(&mut sh, &mut w);
    assert!(text.contains("enable secret wb1$"), "{text}");
    assert!(
        !text.contains("cisco123"),
        "the secret is stored hashed: {text}"
    );
    run(&mut sh, &mut w, &["disable", "enable"]);
    assert_eq!(sh.prompt(&w), PASSWORD_PROMPT);
    assert_eq!(
        sh.help("anything"),
        "",
        "no command help while a password is asked"
    );
    assert_eq!(sh.exec(&mut w, "cisco123"), Some(String::new()));
    assert_eq!(sh.prompt(&w), "r1#");
}

#[test]
fn three_wrong_passwords_give_up_with_a_message() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["enable secret s3cret"]);
    run(&mut sh, &mut w, &["disable", "enable"]);
    let outs = run(&mut sh, &mut w, &["a", "b", "c"]);
    assert_eq!(outs, ["", "", "% Bad secrets"]);
    assert_eq!(sh.prompt(&w), "r1>", "still unprivileged");
    run(&mut sh, &mut w, &["enable", "s3cret"]);
    assert_eq!(sh.prompt(&w), "r1#");
}

#[test]
fn the_secret_beats_the_password() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &["enable password plain", "enable secret strong"],
    );
    run(&mut sh, &mut w, &["disable", "enable", "plain"]);
    assert_eq!(
        sh.prompt(&w),
        PASSWORD_PROMPT,
        "the password no longer works"
    );
    run(&mut sh, &mut w, &["strong"]);
    assert_eq!(sh.prompt(&w), "r1#");
}

#[test]
fn service_password_encryption_changes_how_passwords_print() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["enable password letmein"]);
    assert!(cfg(&mut sh, &mut w).contains("enable password letmein\n"));
    conf(&mut sh, &mut w, &["service password-encryption"]);
    let text = cfg(&mut sh, &mut w);
    assert!(text.starts_with("service password-encryption\n"), "{text}");
    assert!(
        text.contains("enable password wb0$") && !text.contains("letmein"),
        "{text}"
    );
    // The encrypted form is accepted back, so a saved config reloads.
    let hex = text
        .lines()
        .find_map(|l| l.strip_prefix("enable password wb0$"))
        .unwrap()
        .to_string();
    conf(
        &mut sh,
        &mut w,
        &["no enable password", &format!("enable password wb0${hex}")],
    );
    run(&mut sh, &mut w, &["disable", "enable", "letmein"]);
    assert_eq!(sh.prompt(&w), "r1#");
    conf(&mut sh, &mut w, &["no service password-encryption"]);
    assert!(cfg(&mut sh, &mut w).contains("enable password letmein\n"));
}

#[test]
fn odd_looking_words_are_ordinary_passwords() {
    // `w` is a prefix of the encoded forms' names, `wb0$zz` only looks like one: all are just
    // passwords, they print, replay and unlock like any other.
    for pw in ["w", "wb", "wb0", "wb1", "wb0$zz", "wb1$bad", "$", "a1b2"] {
        let (mut w, mut sh) = router();
        conf(
            &mut sh,
            &mut w,
            &[
                &format!("enable secret {pw}"),
                "line console 0",
                &format!("password {pw}"),
            ],
        );
        run(&mut sh, &mut w, &["disable", "enable", pw]);
        assert_eq!(sh.prompt(&w), "r1#", "{pw}");
        let text = cfg(&mut sh, &mut w);
        let mut other = blank_chain();
        let mut sh2 = Shell::router(other.r1);
        assert_eq!(
            sh2.replay(&mut other.world, &text),
            Vec::<String>::new(),
            "{pw}"
        );
        assert_eq!(
            running_config(other.world.device::<Router>(other.r1)),
            text,
            "{pw}"
        );
        assert_eq!(
            other
                .world
                .device::<Router>(other.r1)
                .mgmt
                .console
                .password
                .as_deref(),
            Some(pw),
            "the line password came back as typed"
        );
    }
}

#[test]
fn console_login_gates_a_new_session_only() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "banner motd #Authorised use only#",
            "line console 0",
            "password lab",
            "login",
        ],
    );
    // The configuring session is already in; a new one is greeted and asked.
    let c = blank_chain();
    let _ = c;
    let mut fresh = Shell::router(sh.device);
    let greeting = fresh.greeting(&w);
    assert!(greeting.contains("Authorised use only"), "{greeting}");
    assert!(greeting.contains("User Access Verification"), "{greeting}");
    assert_eq!(fresh.prompt(&w), PASSWORD_PROMPT);
    assert_eq!(fresh.exec(&mut w, "wrong").unwrap(), "");
    assert_eq!(fresh.exec(&mut w, "wrong").unwrap(), "");
    assert_eq!(fresh.exec(&mut w, "wrong").unwrap(), "% Bad passwords");
    assert_eq!(
        fresh.prompt(&w),
        PASSWORD_PROMPT,
        "asks again, never locks out"
    );
    assert_eq!(fresh.exec(&mut w, "lab").unwrap(), "");
    assert_eq!(fresh.prompt(&w), "r1>");
    // Without `login` the password is stored but nobody is asked.
    conf(&mut sh, &mut w, &["line console 0", "no login"]);
    let mut open = Shell::router(sh.device);
    assert!(!open.greeting(&w).contains("User Access"));
    assert_eq!(open.prompt(&w), "r1>");
}

#[test]
fn login_without_a_password_does_not_lock_the_console() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["line console 0", "login"]);
    let mut fresh = Shell::router(sh.device);
    assert_eq!(fresh.greeting(&w), "");
    assert_eq!(fresh.prompt(&w), "r1>");
}

#[test]
fn line_blocks_print_and_replay() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "service password-encryption",
            "line console 0",
            "password conpw",
            "login",
            "exit",
            "line vty 0 15",
            "password vtypw",
            "login",
        ],
    );
    let text = cfg(&mut sh, &mut w);
    assert!(text.contains("line console 0\n password wb0$"), "{text}");
    assert!(text.contains("line vty 0 15\n password wb0$"), "{text}");
    assert!(!text.contains("conpw") && !text.contains("vtypw"), "{text}");
    assert_eq!(text.matches(" login\n").count(), 2);
}

#[test]
fn a_one_line_banner_and_a_multi_line_banner() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["banner motd $ keep out $"]);
    assert!(cfg(&mut sh, &mut w).contains("banner motd #\n keep out \n#\n"));
    run(&mut sh, &mut w, &["enable", "configure terminal"]);
    let first = sh.exec(&mut w, "banner motd #").unwrap();
    assert_eq!(first, "Enter TEXT message.  End with the character '#'.");
    assert_eq!(sh.prompt(&w), "", "no prompt while the text arrives");
    for l in ["line one", "", "! not a comment", "last line#"] {
        assert_eq!(sh.exec(&mut w, l).unwrap(), "");
    }
    assert_eq!(sh.prompt(&w), "r1(config)#");
    sh.exec(&mut w, "end");
    let text = cfg(&mut sh, &mut w);
    assert!(
        text.contains("banner motd #\nline one\n\n! not a comment\nlast line\n#\n"),
        "{text}"
    );
    conf(&mut sh, &mut w, &["no banner motd"]);
    assert!(!cfg(&mut sh, &mut w).contains("banner"));
}

#[test]
fn banner_and_passwords_survive_a_replay_of_the_printed_config() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "hostname edge",
            "service password-encryption",
            "enable secret hunter2",
            "enable password pw2",
            "line vty 0 4",
            "password vtypw",
            "login",
        ],
    );
    run(
        &mut sh,
        &mut w,
        &[
            "enable",
            "configure terminal",
            "banner motd #",
            "a",
            "",
            "! b",
            "#",
            "end",
        ],
    );
    let before = cfg(&mut sh, &mut w);
    // A fresh router of the same shape, configured only by replaying the text.
    let mut other = blank_chain();
    let mut sh2 = Shell::router(other.r1);
    let problems = sh2.replay(&mut other.world, &before);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        running_config(other.world.device::<Router>(other.r1)),
        before
    );
    // The hash is stored as printed, so the same secret still unlocks it.
    run(
        &mut sh2,
        &mut other.world,
        &["end", "disable", "enable", "hunter2"],
    );
    assert_eq!(sh2.prompt(&other.world), "edge#");
}

#[test]
fn write_saves_and_show_startup_config_reads_it_back() {
    let (mut w, mut sh) = router();
    run(&mut sh, &mut w, &["enable"]);
    assert_eq!(
        run(&mut sh, &mut w, &["show startup-config"])[0],
        "startup-config is not present"
    );
    conf(&mut sh, &mut w, &["hostname saved"]);
    for cmd in [
        "write",
        "write memory",
        "copy running-config startup-config",
    ] {
        sh.exec(&mut w, "enable");
        assert_eq!(
            sh.exec(&mut w, cmd).unwrap(),
            "Configuration saved.",
            "{cmd}"
        );
    }
    let start = sh.exec(&mut w, "show startup-config").unwrap();
    assert!(start.starts_with("hostname saved"), "{start}");
    assert_eq!(sh.startup_config(&w), Some(start.as_str()));
    // A configuration changed afterwards is not in the saved one.
    conf(&mut sh, &mut w, &["hostname later"]);
    assert!(sh
        .exec(&mut w, "show startup-config")
        .unwrap()
        .starts_with("hostname saved"));
}

#[test]
fn erase_startup_config_asks_first() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["hostname x"]);
    run(&mut sh, &mut w, &["write"]);
    assert_eq!(sh.exec(&mut w, "erase startup-config").unwrap(), "");
    assert_eq!(sh.prompt(&w), "Erasing the saved configuration. [confirm]");
    assert_eq!(
        sh.exec(&mut w, "n").unwrap(),
        "",
        "anything but yes cancels"
    );
    assert!(sh.startup_config(&w).is_some());
    sh.exec(&mut w, "erase startup-config");
    assert!(sh.exec(&mut w, "").unwrap().contains("complete"));
    assert!(sh.startup_config(&w).is_none());
}

#[test]
fn reload_restores_the_saved_config_and_drops_unsaved_changes() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "hostname saved",
            "interface g0/0",
            "ip address 10.1.1.1 255.255.255.0",
            "no shutdown",
        ],
    );
    run(&mut sh, &mut w, &["write"]);
    conf(
        &mut sh,
        &mut w,
        &["hostname unsaved", "ip route 10.9.0.0 255.255.0.0 10.1.1.2"],
    );
    run(&mut sh, &mut w, &["enable", "reload"]);
    assert_eq!(
        sh.prompt(&w),
        "System configuration has been modified. Save? [yes/no]: "
    );
    assert_eq!(
        sh.exec(&mut w, "maybe").unwrap(),
        "% Please answer 'yes' or 'no'."
    );
    assert_eq!(sh.exec(&mut w, "no").unwrap(), "");
    assert_eq!(sh.prompt(&w), "Proceed with reload? [confirm]");
    let out = sh.exec(&mut w, "").unwrap();
    assert!(
        out.contains("Reload requested.") && out.contains("System restarted."),
        "{out}"
    );
    assert_eq!(sh.prompt(&w), "saved>", "back to user mode, saved hostname");
    let r = w.device::<Router>(sh.device);
    assert!(r.statics().is_empty(), "the unsaved route is gone");
    assert_eq!(
        r.interfaces()[0].addr.map(|a| a.0.to_string()).as_deref(),
        Some("10.1.1.1")
    );
}

#[test]
fn reload_can_save_first_and_can_be_cancelled() {
    let (mut w, mut sh) = router();
    conf(&mut sh, &mut w, &["hostname kept"]);
    run(&mut sh, &mut w, &["enable", "reload", "yes"]);
    assert_eq!(sh.prompt(&w), "Proceed with reload? [confirm]");
    assert_eq!(sh.exec(&mut w, "n").unwrap(), "", "cancelled");
    assert_eq!(sh.prompt(&w), "kept#", "nothing happened");
    run(&mut sh, &mut w, &["reload"]);
    assert_eq!(
        sh.prompt(&w),
        "Proceed with reload? [confirm]",
        "saved, so no save question"
    );
    sh.exec(&mut w, "yes");
    assert_eq!(sh.prompt(&w), "kept>");
}

#[test]
fn reload_without_a_saved_config_is_a_factory_reset() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "hostname gone",
            "enable secret s",
            "interface g0/0",
            "ip address 10.1.1.1 255.255.255.0",
        ],
    );
    run(
        &mut sh,
        &mut w,
        &["disable", "enable", "s", "reload", "no", ""],
    );
    assert_eq!(sh.prompt(&w), "Router>");
    let r = w.device::<Router>(sh.device);
    assert!(r.mgmt.enable_secret.is_none());
    assert!(r.interfaces()[0].addr.is_none() && r.interfaces()[0].shutdown);
}

#[test]
fn reload_shows_the_banner_and_asks_for_the_console_password_again() {
    let (mut w, mut sh) = router();
    conf(
        &mut sh,
        &mut w,
        &[
            "banner motd #welcome#",
            "line console 0",
            "password lab",
            "login",
        ],
    );
    run(&mut sh, &mut w, &["write", "enable", "reload"]);
    let out = sh.exec(&mut w, "").unwrap();
    assert!(
        out.contains("welcome") && out.contains("User Access Verification"),
        "{out}"
    );
    assert_eq!(sh.prompt(&w), PASSWORD_PROMPT);
}

#[test]
fn a_switch_has_the_same_management_plane() {
    let mut lab = load_and_build(ONE_ROUTER, Trace::new()).unwrap();
    let sw = lab.devices[1].id;
    let mut sh = Shell::switch(sw);
    conf(
        &mut sh,
        &mut lab.world,
        &[
            "enable secret sw-secret",
            "vlan 10",
            "banner motd #hi#",
            "line vty 0 4",
            "password v",
            "login",
        ],
    );
    let text = switch_config(lab.world.device::<L2Switch>(sw));
    assert!(
        text.contains("enable secret wb1$") && text.contains("banner motd #\nhi\n#"),
        "{text}"
    );
    run(
        &mut sh,
        &mut lab.world,
        &["write", "disable", "enable", "sw-secret"],
    );
    assert_eq!(sh.prompt(&lab.world), "sw1#");
    // Reload restores VLANs and the secret from the saved config, and the hostname.
    conf(&mut sh, &mut lab.world, &["no vlan 10", "hostname temp"]);
    run(&mut sh, &mut lab.world, &["enable", "reload", "no", ""]);
    assert_eq!(sh.prompt(&lab.world), "sw1>");
    let s = lab.world.device::<L2Switch>(sw);
    assert!(s.vlan_exists(10) && s.mgmt.enable_secret.is_some());
}

#[test]
fn the_saved_config_travels_with_the_lab() {
    let lab = load_and_build(ONE_ROUTER, Trace::new()).unwrap();
    let r1 = lab.devices[0].id;
    let (mut w, topo, _) = lab.into_parts();
    let mut sh = Shell::router(r1);
    conf(
        &mut sh,
        &mut w,
        &["hostname persisted", "enable secret abc"],
    );
    run(&mut sh, &mut w, &["write"]);
    conf(&mut sh, &mut w, &["hostname running-only"]);
    let text = labfile::save(&topo.snapshot(&w));
    assert!(text.contains("startup_config"), "{text}");
    let again = load_and_build(&text, Trace::new()).unwrap();
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    let r = again.world.device::<Router>(again.devices[0].id);
    assert_eq!(
        r.name, "running-only",
        "the running config comes back as it was"
    );
    assert!(r
        .startup
        .as_deref()
        .unwrap()
        .starts_with("hostname persisted"));
    // And reloading that device returns to the saved configuration.
    let mut sh2 = Shell::router(again.devices[0].id);
    let mut w2 = again.world;
    run(&mut sh2, &mut w2, &["enable", "abc", "reload", "no", ""]);
    assert_eq!(sh2.prompt(&w2), "persisted>");
}

mod never_panics {
    use super::*;
    use proptest::prelude::*;

    const POOL: &[&str] = &[
        "enable",
        "disable",
        "configure terminal",
        "end",
        "exit",
        "hostname x",
        "enable secret s",
        "enable secret wb1$00000000$0000000000000000",
        "enable password p",
        "enable password wb0$zz",
        "no enable secret",
        "service password-encryption",
        "no service password-encryption",
        "banner motd #",
        "banner motd $hi$",
        "banner motd",
        "no banner motd",
        "#",
        "line console 0",
        "line vty 0 15",
        "password lab",
        "password wb0$00",
        "no password",
        "login",
        "no login",
        "write",
        "show running-config",
        "show startup-config",
        "erase startup-config",
        "reload",
        "yes",
        "no",
        "y",
        "n",
        "",
        "?",
        "interface g0/0",
        "interface fa0/1",
        "interface vlan 10",
        "vlan 10",
        "wrong",
        "é",
        "password w",
        "enable secret wb",
        "enable password wb0",
    ];

    proptest! {
        /// Any order of management commands and prompt answers is survivable, on both kinds of
        /// device, and the printed config always replays without complaint.
        #[test]
        fn random_sessions_never_panic_and_the_config_replays(
            steps in proptest::collection::vec(0..POOL.len(), 0..60),
            switch in any::<bool>(),
        ) {
            let mut lab = load_and_build(ONE_ROUTER, Trace::new()).unwrap();
            let id = lab.devices[usize::from(switch)].id;
            let mut sh = if switch { Shell::switch(id) } else { Shell::router(id) };
            let _ = sh.greeting(&lab.world);
            for i in steps {
                let _ = sh.prompt(&lab.world);
                let _ = sh.help(POOL[i]);
                let _ = sh.tab(POOL[i]);
                if sh.exec(&mut lab.world, POOL[i]).is_none() {
                    sh = if switch { Shell::switch(id) } else { Shell::router(id) };
                }
            }
            // Whatever state it is in, a fresh shell can print the config and replay it.
            let mut view = if switch { Shell::switch(id) } else { Shell::router(id) };
            view.exec(&mut lab.world, "enable");
            // A secret may now be set; read the config without going through `enable`.
            let text = if switch {
                switch_config(lab.world.device::<L2Switch>(id))
            } else {
                running_config(lab.world.device::<Router>(id))
            };
            let mut again = load_and_build(ONE_ROUTER, Trace::new()).unwrap();
            let mut replayer = if switch {
                Shell::switch(again.devices[1].id)
            } else {
                Shell::router(again.devices[0].id)
            };
            let problems = replayer.replay(&mut again.world, &text);
            prop_assert!(problems.is_empty(), "{problems:?}\n{text}");
            let text2 = if switch {
                switch_config(again.world.device::<L2Switch>(again.devices[1].id))
            } else {
                running_config(again.world.device::<Router>(again.devices[0].id))
            };
            prop_assert_eq!(text2, text);
        }
    }
}
