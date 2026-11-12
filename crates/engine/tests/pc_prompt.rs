//! The PC's command prompt: Windows-style prompt, messages and addressing commands.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::Trace;
use netdev::Host;
use wire::Ipv4Addr;

fn pc() -> (BuiltLab, Shell) {
    let lab = load_and_build(
        include_str!("../../../testkit/scenarios/m3_remote.lab.toml"),
        Trace::new(),
    )
    .unwrap();
    let sh = Shell::host(lab.devices[0].id);
    (lab, sh)
}

#[test]
fn the_prompt_is_a_drive_letter_and_there_is_no_help() {
    let (lab, sh) = pc();
    assert_eq!(sh.prompt(&lab.world), "C:\\>");
    assert_eq!(sh.help("ip"), "");
}

#[test]
fn unknown_commands_get_the_command_prompt_wording() {
    let (mut lab, mut sh) = pc();
    assert_eq!(
        sh.exec(&mut lab.world, "frobnicate now").unwrap(),
        "'frobnicate' is not recognized as an internal or external command,\noperable program or batch file."
    );
    assert_eq!(
        sh.exec(&mut lab.world, "ping").unwrap(),
        "The syntax of the command is incorrect."
    );
    assert_eq!(
        sh.exec(&mut lab.world, "ping not-an-address").unwrap(),
        "Ping request could not find host not-an-address. Please check the name and try again."
    );
    // Something that is neither an address nor a name is still a syntax error.
    assert_eq!(
        sh.exec(&mut lab.world, "ping 1.2.3").unwrap(),
        "The syntax of the command is incorrect."
    );
}

#[test]
fn netsh_sets_a_static_address_with_or_without_a_gateway() {
    let (mut lab, mut sh) = pc();
    let id = lab.devices[0].id;
    sh.exec(
        &mut lab.world,
        "netsh interface ip set address Ethernet static 192.168.5.7 255.255.255.0 192.168.5.1",
    );
    let h = lab.world.device::<Host>(id);
    assert_eq!(
        (h.ip, h.prefix_len, h.gateway),
        (
            Ipv4Addr::new(192, 168, 5, 7),
            24,
            Some(Ipv4Addr::new(192, 168, 5, 1))
        )
    );
    // Without a gateway on the line, there is none afterwards.
    sh.exec(
        &mut lab.world,
        "netsh interface ip set address Ethernet static 10.1.1.2 255.255.0.0",
    );
    let h = lab.world.device::<Host>(id);
    assert_eq!(
        (h.ip, h.prefix_len, h.gateway),
        (Ipv4Addr::new(10, 1, 1, 2), 16, None)
    );
    let out = sh.exec(&mut lab.world, "ipconfig /all").unwrap();
    assert!(
        out.contains("10.1.1.2") && out.contains("255.255.0.0"),
        "{out}"
    );
    let out = sh
        .exec(&mut lab.world, "netsh interface ip show config")
        .unwrap();
    assert!(out.contains("10.1.1.2"), "{out}");
}

#[test]
fn netsh_dhcp_hands_the_address_to_the_dhcp_client() {
    let (mut lab, mut sh) = pc();
    sh.exec(
        &mut lab.world,
        "netsh interface ip set address Ethernet dhcp",
    );
    assert!(lab.world.device::<Host>(lab.devices[0].id).dhcp.enabled);
    sh.exec(&mut lab.world, "ipconfig /release");
    assert_eq!(
        lab.world.device::<Host>(lab.devices[0].id).ip,
        Ipv4Addr::UNSPECIFIED
    );
}

#[test]
fn help_lists_the_commands() {
    let (mut lab, mut sh) = pc();
    let out = sh.exec(&mut lab.world, "help").unwrap();
    for word in ["IPCONFIG", "NETSH", "PING", "TRACERT", "TELNET", "SSH"] {
        assert!(out.contains(word), "{word} missing from:\n{out}");
    }
}

#[test]
fn the_old_ipconfig_forms_still_work() {
    let (mut lab, mut sh) = pc();
    sh.exec(&mut lab.world, "ipconfig set 10.9.9.9 255.255.255.0");
    sh.exec(&mut lab.world, "ipconfig gateway 10.9.9.1");
    let h = lab.world.device::<Host>(lab.devices[0].id);
    assert_eq!(
        (h.ip, h.gateway),
        (Ipv4Addr::new(10, 9, 9, 9), Some(Ipv4Addr::new(10, 9, 9, 1)))
    );
}

#[test]
fn commands_ignore_case_but_user_names_do_not() {
    let (mut lab, mut sh) = pc();
    sh.exec(&mut lab.world, "IPCONFIG SET 10.4.4.4 255.255.255.0");
    assert_eq!(
        lab.world.device::<Host>(lab.devices[0].id).ip,
        Ipv4Addr::new(10, 4, 4, 4)
    );
    let out = sh.exec(&mut lab.world, "Ipconfig /All").unwrap();
    assert!(out.contains("10.4.4.4"), "{out}");
    // `-l Admin` keeps its capital: the prompt for the password appears.
    sh.exec(&mut lab.world, "SSH -L Admin 10.4.4.9");
    assert_eq!(sh.prompt(&lab.world), "Password: ");
}

#[test]
fn cls_asks_the_terminal_to_clear() {
    let (mut lab, mut sh) = pc();
    assert_eq!(
        sh.exec(&mut lab.world, "cls").unwrap(),
        engine::shell::CLEAR_SCREEN
    );
}
