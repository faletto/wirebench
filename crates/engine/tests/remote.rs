//! Telnet and SSH between simulated devices: real TCP connections, vty login, ACLs.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::Trace;

const LAB: &str = include_str!("../../../testkit/scenarios/m3_remote.lab.toml");
const PC: usize = 0;
const R1: usize = 1;
const SW1: usize = 2;
const R2: usize = 3;

struct Term {
    sh: Shell,
    dev: usize,
}

fn open(lab: &BuiltLab, dev: usize) -> Term {
    let id = lab.devices[dev].id;
    let sh = match dev {
        PC => Shell::host(id),
        SW1 => Shell::switch(id),
        _ => Shell::router(id),
    };
    Term { sh, dev }
}

impl Term {
    fn run(&mut self, lab: &mut BuiltLab, lines: &[&str]) -> String {
        let mut out = String::new();
        for l in lines {
            out = self
                .sh
                .exec(&mut lab.world, l)
                .unwrap_or_else(|| "<ended>".into());
        }
        out
    }
    fn prompt(&self, lab: &BuiltLab) -> String {
        self.sh.prompt(&lab.world)
    }
    fn dev(&self) -> usize {
        self.dev
    }
}

fn setup() -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    for (d, ip) in [(R1, "10.0.0.1"), (R2, "10.0.0.2")] {
        let mut t = open(&lab, d);
        t.run(
            &mut lab,
            &[
                "enable",
                "configure terminal",
                "interface g0/0",
                &format!("ip address {ip} 255.255.255.0"),
                "no shutdown",
                "end",
            ],
        );
    }
    let mut t = open(&lab, SW1);
    t.run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "end",
        ],
    );
    lab
}

fn set_vty_password(lab: &mut BuiltLab, dev: usize, pw: &str) {
    open(lab, dev).run(
        lab,
        &[
            "enable",
            "configure terminal",
            "line vty 0 4",
            &format!("password {pw}"),
            "login",
            "end",
        ],
    );
}

fn enable_ssh(lab: &mut BuiltLab, dev: usize) {
    open(lab, dev).run(
        lab,
        &[
            "enable",
            "configure terminal",
            "ip domain-name lab.example",
            "crypto key generate rsa",
            "username admin privilege 15 secret s3cret",
            "username guest password visitor",
            "line vty 0 4",
            "login local",
            "end",
        ],
    );
}

#[test]
fn telnet_with_the_line_password_reaches_the_far_prompt_and_exit_comes_back() {
    let mut lab = setup();
    set_vty_password(&mut lab, R2, "cisco");
    let mut t = open(&lab, R1);
    t.run(&mut lab, &["enable"]);
    let out = t.run(&mut lab, &["telnet 10.0.0.2"]);
    assert!(out.starts_with("Trying 10.0.0.2 ... Open"), "{out}");
    assert!(out.contains("User Access Verification"), "{out}");
    assert_eq!(t.prompt(&lab), "Password: ");
    // Wrong, then right.
    t.run(&mut lab, &["nope"]);
    assert_eq!(t.prompt(&lab), "Password: ");
    t.run(&mut lab, &["cisco"]);
    assert_eq!(t.prompt(&lab), "r2>");
    // Commands run on r2.
    let out = t.run(&mut lab, &["enable", "show ip interface brief"]);
    assert!(out.contains("10.0.0.2"), "{out}");
    assert_eq!(t.prompt(&lab), "r2#");
    t.run(&mut lab, &["configure terminal", "hostname far", "end"]);
    assert_eq!(t.prompt(&lab), "far#");
    // exit from privileged goes down a level, exit again closes the session.
    let out = t.run(&mut lab, &["disable", "exit"]);
    assert!(
        out.contains("[Connection to 10.0.0.2 closed by foreign host]"),
        "{out}"
    );
    assert_eq!(t.prompt(&lab), "r1#");
    // The change really happened on the other router.
    let r2 = lab.world.device::<netdev::Router>(lab.devices[R2].id);
    assert_eq!(r2.name, "far");
}

#[test]
fn a_line_with_login_but_no_password_refuses_to_let_anyone_in() {
    let mut lab = setup();
    open(&lab, R2).run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "line vty 0 4",
            "login",
            "end",
        ],
    );
    let mut t = open(&lab, R1);
    let out = t.run(&mut lab, &["telnet 10.0.0.2"]);
    assert!(out.contains("Password required, but none set"), "{out}");
    assert!(out.contains("closed by foreign host"), "{out}");
    assert_eq!(t.prompt(&lab), "r1>");
}

#[test]
fn a_line_without_login_lets_you_straight_in() {
    let mut lab = setup();
    let mut t = open(&lab, R1);
    t.run(&mut lab, &["telnet 10.0.0.2"]);
    assert_eq!(t.prompt(&lab), "r2>");
}

#[test]
fn three_wrong_passwords_close_the_connection() {
    let mut lab = setup();
    set_vty_password(&mut lab, R2, "cisco");
    let mut t = open(&lab, R1);
    t.run(&mut lab, &["telnet 10.0.0.2", "a", "b"]);
    let out = t.run(&mut lab, &["c"]);
    assert!(out.contains("% Bad passwords"), "{out}");
    assert!(out.contains("closed by foreign host"), "{out}");
    assert_eq!(t.prompt(&lab), "r1>");
}

#[test]
fn a_pc_can_telnet_to_a_router_and_a_switch() {
    let mut lab = setup();
    set_vty_password(&mut lab, R1, "pw1");
    set_vty_password(&mut lab, SW1, "pw3");
    let mut t = open(&lab, PC);
    t.run(&mut lab, &["telnet 10.0.0.1", "pw1"]);
    assert_eq!(t.prompt(&lab), "r1>");
    t.run(&mut lab, &["exit"]);
    assert_eq!(t.prompt(&lab), "pc1>");
    let out = t.run(&mut lab, &["telnet 10.0.0.3", "pw3"]);
    assert_eq!(t.prompt(&lab), "sw1>", "{out}");
    let out = t.run(&mut lab, &["show ip ssh"]);
    assert!(out.starts_with("SSH Disabled"), "{out}");
}

#[test]
fn sessions_nest_through_a_switch_to_a_router() {
    let mut lab = setup();
    set_vty_password(&mut lab, SW1, "swpw");
    set_vty_password(&mut lab, R2, "r2pw");
    let mut t = open(&lab, R1);
    t.run(
        &mut lab,
        &["telnet 10.0.0.3", "swpw", "telnet 10.0.0.2", "r2pw"],
    );
    assert_eq!(t.prompt(&lab), "r2>");
    // Leaving the inner session lands in the switch, then the outer one in r1.
    let out = t.run(&mut lab, &["exit"]);
    assert!(out.contains("10.0.0.2 closed"), "{out}");
    assert_eq!(t.prompt(&lab), "sw1>");
    t.run(&mut lab, &["exit"]);
    assert_eq!(t.prompt(&lab), "r1>");
    assert_eq!(t.dev(), R1);
}

#[test]
fn ssh_needs_a_key_and_a_domain_name_on_the_far_end() {
    let mut lab = setup();
    let mut t = open(&lab, R1);
    assert_eq!(
        t.run(&mut lab, &["ssh -l admin 10.0.0.2", "s3cret"]),
        "% Connection refused by remote host"
    );
    let mut far = open(&lab, R2);
    assert_eq!(
        far.run(
            &mut lab,
            &["enable", "configure terminal", "crypto key generate rsa"]
        ),
        "% Please define a domain-name first."
    );
    let out = far.run(
        &mut lab,
        &["ip domain-name lab.example", "crypto key generate rsa"],
    );
    assert!(
        out.starts_with("The name for the keys will be: r2.lab.example"),
        "{out}"
    );
    assert!(out.contains("1024 bit"), "{out}");
}

#[test]
fn ssh_logs_in_with_a_local_user_and_privilege_15_lands_in_enable_mode() {
    let mut lab = setup();
    enable_ssh(&mut lab, R2);
    let mut t = open(&lab, R1);
    let out = t.run(&mut lab, &["ssh -l admin 10.0.0.2"]);
    assert_eq!(out, "");
    assert_eq!(t.prompt(&lab), "Password: ");
    let out = t.run(&mut lab, &["s3cret"]);
    assert!(out.starts_with("Trying 10.0.0.2 ... Open"), "{out}");
    assert_eq!(t.prompt(&lab), "r2#");
    let out = t.run(&mut lab, &["show ip ssh"]);
    assert!(out.starts_with("SSH Enabled - version 2.0"), "{out}");
    t.run(&mut lab, &["exit"]);
    assert_eq!(t.prompt(&lab), "r1>");

    // A privilege-1 user lands in user mode.
    t.run(&mut lab, &["ssh -l guest 10.0.0.2", "visitor"]);
    assert_eq!(t.prompt(&lab), "r2>");
}

#[test]
fn ssh_with_a_wrong_password_fails_and_leaves_you_where_you_were() {
    let mut lab = setup();
    enable_ssh(&mut lab, R2);
    let mut t = open(&lab, R1);
    let out = t.run(&mut lab, &["ssh -l admin 10.0.0.2", "wrong"]);
    assert_eq!(out, "% Authentication failed.");
    assert_eq!(t.prompt(&lab), "r1>");
    let out = t.run(&mut lab, &["ssh -l nobody 10.0.0.2", "x"]);
    assert_eq!(out, "% Authentication failed.");
}

#[test]
fn telnet_to_a_login_local_line_asks_for_username_then_password() {
    let mut lab = setup();
    enable_ssh(&mut lab, R2);
    let mut t = open(&lab, R1);
    t.run(&mut lab, &["telnet 10.0.0.2"]);
    assert_eq!(t.prompt(&lab), "Username: ");
    t.run(&mut lab, &["admin"]);
    assert_eq!(t.prompt(&lab), "Password: ");
    let out = t.run(&mut lab, &["bad"]);
    assert_eq!(out, "% Login invalid");
    assert_eq!(t.prompt(&lab), "Username: ");
    t.run(&mut lab, &["admin", "s3cret"]);
    assert_eq!(t.prompt(&lab), "r2#");
}

#[test]
fn transport_input_decides_which_protocol_is_listening() {
    let mut lab = setup();
    enable_ssh(&mut lab, R2);
    let mut far = open(&lab, R2);
    far.run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "line vty 0 4",
            "transport input ssh",
            "end",
        ],
    );
    let mut t = open(&lab, R1);
    assert_eq!(
        t.run(&mut lab, &["telnet 10.0.0.2"]),
        "% Connection refused by remote host"
    );
    t.run(&mut lab, &["ssh -l admin 10.0.0.2", "s3cret"]);
    assert_eq!(t.prompt(&lab), "r2#");
    t.run(&mut lab, &["exit"]);
    far.run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "line vty 0 4",
            "transport input none",
            "end",
        ],
    );
    assert_eq!(
        t.run(&mut lab, &["ssh -l admin 10.0.0.2", "s3cret"]),
        "% Connection refused by remote host"
    );
}

#[test]
fn access_class_on_the_vty_lines_refuses_the_callers_it_denies() {
    let mut lab = setup();
    set_vty_password(&mut lab, R2, "cisco");
    open(&lab, R2).run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "access-list 10 permit 10.0.0.10",
            "line vty 0 4",
            "access-class 10 in",
            "end",
        ],
    );
    // r1 (10.0.0.1) is not on the list; the PC (10.0.0.10) is.
    let mut r1 = open(&lab, R1);
    assert_eq!(
        r1.run(&mut lab, &["telnet 10.0.0.2"]),
        "% Connection refused by remote host"
    );
    assert_eq!(r1.prompt(&lab), "r1>");
    let mut pc = open(&lab, PC);
    pc.run(&mut lab, &["telnet 10.0.0.2", "cisco"]);
    assert_eq!(pc.prompt(&lab), "r2>");
}

#[test]
fn nothing_answering_times_out_and_says_so() {
    let mut lab = setup();
    let mut t = open(&lab, R1);
    let out = t.run(&mut lab, &["telnet 10.0.0.77"]);
    assert_eq!(out, "% Connection timed out; remote host not responding");
    assert_eq!(
        t.run(&mut lab, &["telnet 192.168.9.9"]),
        "% Destination unreachable; gateway or host down"
    );
}

#[test]
fn show_tcp_brief_lists_the_open_session_on_both_ends() {
    let mut lab = setup();
    let mut t = open(&lab, R1);
    t.run(&mut lab, &["telnet 10.0.0.2"]);
    let far = open(&lab, R2).run(&mut lab, &["show tcp brief"]);
    assert!(
        far.contains("10.0.0.2.23") && far.contains("10.0.0.1.") && far.contains("ESTAB"),
        "{far}"
    );
    // Through the session itself too.
    let near = t.run(&mut lab, &["show tcp brief"]);
    assert!(near.contains("ESTAB"), "{near}");
    t.run(&mut lab, &["exit"]);
    let far = open(&lab, R2).run(&mut lab, &["show tcp brief"]);
    assert!(!far.contains("ESTAB"), "{far}");
}

#[test]
fn users_domain_and_transport_are_in_the_running_config_and_survive_a_reload() {
    let mut lab = setup();
    enable_ssh(&mut lab, R2);
    let mut far = open(&lab, R2);
    far.run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "line vty 0 4",
            "transport input ssh",
            "end",
        ],
    );
    let cfg = far.run(&mut lab, &["show running-config"]);
    assert!(cfg.contains("ip domain-name lab.example"), "{cfg}");
    assert!(
        cfg.contains("username admin privilege 15 secret wb1$"),
        "{cfg}"
    );
    assert!(cfg.contains("username guest password visitor"), "{cfg}");
    assert!(cfg.contains(" login local"), "{cfg}");
    assert!(cfg.contains(" transport input ssh"), "{cfg}");
    assert!(!cfg.contains("s3cret"), "{cfg}");
    far.run(&mut lab, &["copy running-config startup-config", "reload"]);
    far.run(&mut lab, &["no", "yes"]);
    let mut far = open(&lab, R2);
    let mut r1 = open(&lab, R1);
    // The config came back, and so did the key (it is not part of the config).
    far.run(
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.0.2 255.255.255.0",
            "no shutdown",
            "end",
        ],
    );
    r1.run(&mut lab, &["ssh -l admin 10.0.0.2", "s3cret"]);
    assert_eq!(r1.prompt(&lab), "r2#");
}

#[test]
fn the_same_session_gives_the_same_trace() {
    let run = || {
        let mut lab = load_and_build(LAB, Trace::keeping()).unwrap();
        for (d, ip) in [(R1, "10.0.0.1"), (R2, "10.0.0.2")] {
            open(&lab, d).run(
                &mut lab,
                &[
                    "enable",
                    "configure terminal",
                    "interface g0/0",
                    &format!("ip address {ip} 255.255.255.0"),
                    "no shutdown",
                    "end",
                ],
            );
        }
        let mut t = open(&lab, R1);
        t.run(&mut lab, &["telnet 10.0.0.2", "exit"]);
        lab.world.trace_hash()
    };
    assert_eq!(run(), run());
}
