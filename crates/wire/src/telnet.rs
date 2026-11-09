//! Telnet (RFC 854) command and option handling (RFC 855), enough for a line-mode session:
//! splitting a byte stream into data and IAC commands, doubling 0xff in data, and answering
//! option negotiation without looping.

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;

/// One negotiation command: `verb` is [`WILL`], [`WONT`], [`DO`] or [`DONT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Negotiation {
    pub verb: u8,
    pub option: u8,
}

impl Negotiation {
    pub fn bytes(self) -> [u8; 3] {
        [IAC, self.verb, self.option]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Data,
    Iac,
    Verb(u8),
    /// Inside a subnegotiation, which is skipped.
    Sub,
    SubIac,
}

/// Splits a stream into user data and negotiations; keeps its place between calls because a
/// command can be cut by a segment boundary.
#[derive(Clone, Debug)]
pub struct Decoder {
    mode: Mode,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder { mode: Mode::Data }
    }
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, Vec<Negotiation>) {
        let (mut data, mut cmds) = (vec![], vec![]);
        for &b in bytes {
            self.mode = match (self.mode, b) {
                (Mode::Data, IAC) => Mode::Iac,
                (Mode::Data, _) => {
                    data.push(b);
                    Mode::Data
                }
                (Mode::Iac, IAC) => {
                    data.push(IAC);
                    Mode::Data
                }
                (Mode::Iac, WILL | WONT | DO | DONT) => Mode::Verb(b),
                (Mode::Iac, SB) => Mode::Sub,
                // Any other command (NOP, AYT...) is ignored.
                (Mode::Iac, _) => Mode::Data,
                (Mode::Verb(verb), option) => {
                    cmds.push(Negotiation { verb, option });
                    Mode::Data
                }
                (Mode::Sub, IAC) => Mode::SubIac,
                (Mode::Sub, _) => Mode::Sub,
                (Mode::SubIac, SE) => Mode::Data,
                (Mode::SubIac, _) => Mode::Sub,
            };
        }
        (data, cmds)
    }
}

/// Double any 0xff so it is not read as a command.
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(data.len());
    for &b in data {
        o.push(b);
        if b == IAC {
            o.push(IAC);
        }
    }
    o
}

/// Which options each side has switched on. Answers a request only when it changes something,
/// which is what stops two ends from acknowledging each other forever (RFC 1143 in short).
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Options we perform (we sent WILL, or they asked DO and we agreed).
    pub us: Vec<u8>,
    /// Options they perform.
    pub them: Vec<u8>,
}

impl Options {
    /// Ask to start performing `option` ourselves. None if it already is.
    pub fn offer(&mut self, option: u8) -> Option<Negotiation> {
        if self.us.contains(&option) {
            return None;
        }
        self.us.push(option);
        Some(Negotiation { verb: WILL, option })
    }

    /// Ask the other end to perform `option`.
    pub fn request(&mut self, option: u8) -> Option<Negotiation> {
        if self.them.contains(&option) {
            return None;
        }
        self.them.push(option);
        Some(Negotiation { verb: DO, option })
    }

    /// What to answer to `n`, given the options we are willing to take part in.
    pub fn answer(&mut self, n: Negotiation, supported: &[u8]) -> Option<Negotiation> {
        let ok = supported.contains(&n.option);
        match n.verb {
            DO if ok && !self.us.contains(&n.option) => {
                self.us.push(n.option);
                Some(Negotiation {
                    verb: WILL,
                    option: n.option,
                })
            }
            DO if !ok => Some(Negotiation {
                verb: WONT,
                option: n.option,
            }),
            WILL if ok && !self.them.contains(&n.option) => {
                self.them.push(n.option);
                Some(Negotiation {
                    verb: DO,
                    option: n.option,
                })
            }
            WILL if !ok => Some(Negotiation {
                verb: DONT,
                option: n.option,
            }),
            DONT => {
                self.us.retain(|o| *o != n.option);
                None
            }
            WONT => {
                self.them.retain(|o| *o != n.option);
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_and_commands_are_separated_even_when_cut() {
        let mut d = Decoder::default();
        let (data, cmds) = d.feed(&[b'h', IAC, DO]);
        assert_eq!((data.as_slice(), cmds.len()), (&b"h"[..], 0));
        let (data, cmds) = d.feed(&[OPT_ECHO, b'i', IAC, IAC, b'!']);
        assert_eq!(data, vec![b'i', 255, b'!']);
        assert_eq!(
            cmds,
            vec![Negotiation {
                verb: DO,
                option: OPT_ECHO
            }]
        );
    }

    #[test]
    fn subnegotiations_are_skipped() {
        let mut d = Decoder::default();
        let (data, cmds) = d.feed(&[IAC, SB, 24, 0, b'x', b't', IAC, SE, b'a']);
        assert_eq!(data, b"a");
        assert!(cmds.is_empty());
    }

    #[test]
    fn escape_round_trips() {
        let raw = [1, 255, 2, 255, 255];
        let mut d = Decoder::default();
        assert_eq!(d.feed(&escape(&raw)).0, raw);
    }

    #[test]
    fn negotiation_settles_without_looping() {
        let mut server = Options::default();
        let mut client = Options::default();
        let supported = [OPT_ECHO, OPT_SGA];
        let mut to_client = vec![
            server.offer(OPT_ECHO).unwrap(),
            server.offer(OPT_SGA).unwrap(),
        ];
        let mut to_server = vec![];
        for _ in 0..4 {
            for n in to_client.drain(..) {
                to_server.extend(client.answer(n, &supported));
            }
            for n in to_server.drain(..) {
                to_client.extend(server.answer(n, &supported));
            }
        }
        assert!(to_client.is_empty() && to_server.is_empty());
        assert_eq!(server.us, vec![OPT_ECHO, OPT_SGA]);
        assert_eq!(client.them, vec![OPT_ECHO, OPT_SGA]);
    }

    #[test]
    fn unknown_options_are_refused() {
        let mut o = Options::default();
        let r = o.answer(
            Negotiation {
                verb: DO,
                option: 24,
            },
            &[OPT_ECHO],
        );
        assert_eq!(r.unwrap().verb, WONT);
        let r = o.answer(
            Negotiation {
                verb: WILL,
                option: 24,
            },
            &[OPT_ECHO],
        );
        assert_eq!(r.unwrap().verb, DONT);
    }
}
