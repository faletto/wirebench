//! TCP and UDP sockets on hosts, end to end through the event loop.

use engine::World;
use kernel::{LinkConfig, SimTime, Trace};
use netdev::{Host, TcpFailure, TcpState};
use wire::{Ipv4Addr, MacAddr};

fn host(n: u8) -> Host {
    Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, 0, n]),
        Ipv4Addr::new(10, 0, 0, n),
        24,
    )
}

fn two_hosts() -> (World, kernel::DeviceId, kernel::DeviceId) {
    let mut w = World::new(Trace::keeping());
    let a = w.add_device(host(1));
    let b = w.add_device(host(2));
    w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
    (w, a, b)
}

const B_IP: Ipv4Addr = Ipv4Addr([10, 0, 0, 2]);

#[test]
fn a_client_connects_sends_and_gets_the_answer() {
    let (mut w, a, b) = two_hosts();
    w.with_device::<Host, _>(b, |h, _| h.tcp.set_listeners(&[7]));
    let ca = w
        .with_device::<Host, _>(a, |h, c| h.tcp_open(c, B_IP, 7))
        .unwrap();
    w.run_to_idle();
    assert_eq!(w.device::<Host>(a).tcp.state_of(ca), Some(TcpState::Established));
    let cb = w.with_device::<Host, _>(b, |h, _| h.tcp.accept()).unwrap();
    w.with_device::<Host, _>(a, |h, c| h.tcp_write(c, ca, b"hello"));
    w.run_to_idle();
    let got = w.with_device::<Host, _>(b, |h, c| h.tcp_take(c, cb));
    assert_eq!(got, b"hello");
    // The server echoes it back, shouting.
    w.with_device::<Host, _>(b, |h, c| h.tcp_write(c, cb, &got.to_ascii_uppercase()));
    w.run_to_idle();
    assert_eq!(w.with_device::<Host, _>(a, |h, c| h.tcp_take(c, ca)), b"HELLO");
}

#[test]
fn closing_ends_both_sides() {
    let (mut w, a, b) = two_hosts();
    w.with_device::<Host, _>(b, |h, _| h.tcp.set_listeners(&[7]));
    let ca = w
        .with_device::<Host, _>(a, |h, c| h.tcp_open(c, B_IP, 7))
        .unwrap();
    w.run_to_idle();
    let cb = w.with_device::<Host, _>(b, |h, _| h.tcp.accept()).unwrap();
    w.with_device::<Host, _>(a, |h, c| h.tcp_shut(c, ca));
    w.run_to_idle();
    assert!(w.device::<Host>(b).tcp.conn(cb).unwrap().peer_closed);
    w.with_device::<Host, _>(b, |h, c| h.tcp_shut(c, cb));
    w.run_to_idle();
    // The server's side is gone; the client's lingers briefly, then goes too.
    assert_eq!(w.device::<Host>(b).tcp.state_of(cb), None);
    let later = w.kernel.now() + SimTime::from_secs(10);
    w.run_until(later);
    assert_eq!(w.device::<Host>(a).tcp.state_of(ca), None);
}

#[test]
fn a_port_nobody_listens_on_refuses() {
    let (mut w, a, _b) = two_hosts();
    let ca = w
        .with_device::<Host, _>(a, |h, c| h.tcp_open(c, B_IP, 80))
        .unwrap();
    w.run_to_idle();
    let c = w.device::<Host>(a).tcp.conn(ca).unwrap();
    assert_eq!(c.state, TcpState::Closed);
    assert_eq!(c.failure, Some(TcpFailure::Refused));
}

#[test]
fn a_dead_cable_times_out_instead_of_hanging() {
    let (mut w, a, _b) = two_hosts();
    // Nothing at 10.0.0.9: ARP never answers, so the SYN never leaves.
    let ca = w
        .with_device::<Host, _>(a, |h, c| h.tcp_open(c, Ipv4Addr::new(10, 0, 0, 9), 23))
        .unwrap();
    let later = w.kernel.now() + SimTime::from_secs(60);
    w.run_until(later);
    assert_eq!(
        w.device::<Host>(a).tcp.conn(ca).unwrap().failure,
        Some(TcpFailure::TimedOut)
    );
}

#[test]
fn a_big_write_arrives_whole_and_in_order() {
    let (mut w, a, b) = two_hosts();
    w.with_device::<Host, _>(b, |h, _| h.tcp.set_listeners(&[7]));
    let ca = w
        .with_device::<Host, _>(a, |h, c| h.tcp_open(c, B_IP, 7))
        .unwrap();
    w.run_to_idle();
    let cb = w.with_device::<Host, _>(b, |h, _| h.tcp.accept()).unwrap();
    let data: Vec<u8> = (0..20_000u32).map(|i| (i % 253) as u8).collect();
    let mut got = vec![];
    w.with_device::<Host, _>(a, |h, c| h.tcp_write(c, ca, &data));
    // The receive window is smaller than the write: the reader has to keep up.
    for _ in 0..50 {
        w.run_to_idle();
        got.extend(w.with_device::<Host, _>(b, |h, c| h.tcp_take(c, cb)));
        if got.len() == data.len() {
            break;
        }
    }
    assert_eq!(got, data);
}

#[test]
fn udp_sockets_receive_and_unbound_ports_get_unreachable() {
    let (mut w, a, b) = two_hosts();
    w.with_device::<Host, _>(b, |h, _| h.tcp.udp_bind(5000));
    w.with_device::<Host, _>(a, |h, c| h.udp_send(c, B_IP, 4000, 5000, b"datagram"));
    w.with_device::<Host, _>(a, |h, c| h.udp_send(c, B_IP, 4000, 5001, b"nobody"));
    w.run_to_idle();
    let got = w.with_device::<Host, _>(b, |h, _| h.tcp.udp_recv(5000)).unwrap();
    assert_eq!(got, (Ipv4Addr::new(10, 0, 0, 1), 4000, b"datagram".to_vec()));
    assert_eq!(w.device::<Host>(a).icmp_errors.len(), 1);
}

#[test]
fn the_same_run_gives_the_same_trace() {
    let run = || {
        let (mut w, a, b) = two_hosts();
        w.with_device::<Host, _>(b, |h, _| h.tcp.set_listeners(&[7]));
        let ca = w
            .with_device::<Host, _>(a, |h, c| h.tcp_open(c, B_IP, 7))
            .unwrap();
        w.run_to_idle();
        w.with_device::<Host, _>(a, |h, c| h.tcp_write(c, ca, b"x"));
        w.run_to_idle();
        w.trace_hash()
    };
    assert_eq!(run(), run());
}
