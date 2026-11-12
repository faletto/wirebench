//! DNS: what a name server answers from a table of names, and a device's resolver settings
//! (`ip name-server`, `ip host`, `ip domain-lookup`). The client that asks is in the shell, where
//! it can wait for the answer.

use std::collections::BTreeMap;
use wire::{
    DnsData, DnsMessage, DnsRecord, Ipv4Addr, DNS_CLASS_IN, DNS_FORMERR, DNS_NOTIMP, DNS_NXDOMAIN,
    DNS_OK, DNS_TYPE_A,
};

/// Seconds a server tells clients they may keep an answer.
pub const ANSWER_TTL: u32 = 300;

/// A name as it is looked up: lower case, no trailing dot.
pub fn canonical(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// The reply to the query in `data`, or None if it is not one (a response, or not DNS at all).
pub fn answer(names: &BTreeMap<String, Ipv4Addr>, data: &[u8]) -> Option<Vec<u8>> {
    let q = DnsMessage::parse(data).ok()?;
    if q.response {
        return None;
    }
    if q.opcode != 0 {
        return Some(DnsMessage::reply(&q, DNS_NOTIMP, vec![]).emit());
    }
    let [question] = q.questions.as_slice() else {
        return Some(DnsMessage::reply(&q, DNS_FORMERR, vec![]).emit());
    };
    let name = canonical(&question.name);
    let Some(addr) = names.get(&name) else {
        return Some(DnsMessage::reply(&q, DNS_NXDOMAIN, vec![]).emit());
    };
    // The name exists. Only A records are kept, so any other type is an empty, successful answer.
    let answers = if question.qtype == DNS_TYPE_A && question.qclass == DNS_CLASS_IN {
        vec![DnsRecord {
            name: question.name.clone(),
            rtype: DNS_TYPE_A,
            class: DNS_CLASS_IN,
            ttl: ANSWER_TTL,
            data: DnsData::A(*addr),
        }]
    } else {
        vec![]
    };
    Some(DnsMessage::reply(&q, DNS_OK, answers).emit())
}

/// A name server on a PC (`dnsd start`).
#[derive(Clone, Debug, Default)]
pub struct DnsServer {
    pub running: bool,
    pub names: BTreeMap<String, Ipv4Addr>,
    /// Queries answered.
    pub queries: u32,
}

/// A router's or switch's resolver settings.
#[derive(Clone, Debug)]
pub struct Resolver {
    /// `ip name-server`, in the order they are asked.
    pub servers: Vec<Ipv4Addr>,
    /// `ip domain-lookup`: on unless `no ip domain-lookup`.
    pub lookup: bool,
    /// `ip host <name> <address>`.
    pub hosts: BTreeMap<String, Ipv4Addr>,
    /// `ip dns server`: answer queries from `hosts`.
    pub serve: bool,
}

impl Default for Resolver {
    fn default() -> Self {
        Resolver {
            servers: Vec::new(),
            lookup: true,
            hosts: BTreeMap::new(),
            serve: false,
        }
    }
}

/// Most name servers a device keeps.
pub const MAX_SERVERS: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> BTreeMap<String, Ipv4Addr> {
        BTreeMap::from([("web.example.com".to_string(), Ipv4Addr::new(10, 0, 0, 5))])
    }

    fn ask(q: &DnsMessage) -> DnsMessage {
        DnsMessage::parse(&answer(&names(), &q.emit()).unwrap()).unwrap()
    }

    #[test]
    fn a_known_name_gets_its_address_whatever_its_case() {
        let r = ask(&DnsMessage::query(7, "WEB.Example.com."));
        assert!(r.response && r.id == 7 && r.rcode == DNS_OK && r.authoritative);
        assert_eq!(r.answers.len(), 1);
        assert_eq!(r.answers[0].data, DnsData::A(Ipv4Addr::new(10, 0, 0, 5)));
        assert_eq!(r.questions.len(), 1);
    }

    #[test]
    fn an_unknown_name_is_nxdomain_and_another_type_of_a_known_one_is_empty() {
        let r = ask(&DnsMessage::query(1, "nope.example.com"));
        assert_eq!((r.rcode, r.answers.len()), (DNS_NXDOMAIN, 0));
        let mut q = DnsMessage::query(2, "web.example.com");
        q.questions[0].qtype = 28;
        let r = ask(&q);
        assert_eq!((r.rcode, r.answers.len()), (DNS_OK, 0));
    }

    #[test]
    fn odd_queries_get_errors_and_noise_gets_nothing() {
        let mut q = DnsMessage::query(3, "x");
        q.opcode = 2;
        assert_eq!(ask(&q).rcode, DNS_NOTIMP);
        let mut q = DnsMessage::query(4, "x");
        q.questions.clear();
        assert_eq!(ask(&q).rcode, DNS_FORMERR);
        let mut resp = DnsMessage::query(5, "web.example.com");
        resp.response = true;
        assert!(answer(&names(), &resp.emit()).is_none());
        assert!(answer(&names(), &[1, 2, 3]).is_none());
    }
}
