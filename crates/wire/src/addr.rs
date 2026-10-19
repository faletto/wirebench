use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);

    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }

    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 == 1
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}{:02x}.{:02x}{:02x}.{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

impl fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const UNSPECIFIED: Ipv4Addr = Ipv4Addr([0; 4]);
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255; 4]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn from_u32(v: u32) -> Self {
        Ipv4Addr(v.to_be_bytes())
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(f, "{}.{}.{}.{}", b[0], b[1], b[2], b[3])
    }
}

impl fmt::Debug for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
