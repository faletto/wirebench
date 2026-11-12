//! Every OSPF packet FRR's ospfd sent or heard in the networks of `testkit/frr-oracle/cases`
//! (recorded by `run.py` into `captures/`) must parse, which checks the packet checksum and the
//! Fletcher checksum of every LSA in them, and must emit again as the same bytes.

use wire::{OspfBody, OspfPacket};

/// The OSPF packets of a pcap file taken with `tcpdump -i any` (Linux cooked capture v2).
fn packets(pcap: &[u8]) -> Vec<Vec<u8>> {
    assert_eq!(&pcap[..4], &[0xd4, 0xc3, 0xb2, 0xa1], "little endian pcap");
    assert_eq!(
        u32::from_le_bytes(pcap[20..24].try_into().unwrap()),
        276,
        "SLL2"
    );
    let mut out = Vec::new();
    let mut at = 24;
    while at + 16 <= pcap.len() {
        let len = u32::from_le_bytes(pcap[at + 8..at + 12].try_into().unwrap()) as usize;
        let frame = &pcap[at + 16..at + 16 + len];
        at += 16 + len;
        let ip = &frame[20..];
        assert_eq!(ip[0] >> 4, 4);
        assert_eq!(ip[9], 89, "OSPF");
        let ihl = (ip[0] & 15) as usize * 4;
        let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
        out.push(ip[ihl..total].to_vec());
    }
    out
}

#[test]
fn every_packet_frr_sent_parses_and_emits_unchanged() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testkit/frr-oracle/captures");
    let (mut files, mut total, mut lsas, mut kinds) = (0, 0, 0, [0usize; 5]);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("pcap") {
            continue;
        }
        files += 1;
        for raw in packets(&std::fs::read(&path).unwrap()) {
            let p =
                OspfPacket::parse(&raw).unwrap_or_else(|e| panic!("{path:?}: {e:?}\n{raw:02x?}"));
            assert_eq!(p.emit(), raw, "{path:?}: re-emitted bytes differ");
            total += 1;
            match &p.body {
                OspfBody::Hello(_) => kinds[0] += 1,
                OspfBody::Dbd(_) => kinds[1] += 1,
                OspfBody::Request(_) => kinds[2] += 1,
                OspfBody::Update(u) => {
                    kinds[3] += 1;
                    lsas += u.len();
                }
                OspfBody::Ack(_) => kinds[4] += 1,
            }
        }
    }
    assert!(files >= 15, "captures are missing");
    eprintln!(
        "{total} packets {kinds:?} (hello, dbd, request, update, ack), {lsas} LSAs in updates"
    );
    assert!(kinds.iter().all(|&k| k > 0) && lsas > 0);
}
