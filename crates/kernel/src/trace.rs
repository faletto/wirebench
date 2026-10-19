use crate::SimTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TraceKind {
    FrameTx = 1,
    FrameRx = 2,
    FrameDrop = 3,
    Timer = 4,
    /// Free-form record emitted by a device (e.g. "ping reply").
    Note = 5,
}

/// Canonical trace. Each record is hashed as
/// `time:u64be | kind:u8 | device:u32be | port:u16be | len:u32be | bytes`
/// with FNV-1a 64, which is trivially portable and has no platform dependence.
#[derive(Debug, Clone)]
pub struct Trace {
    hash: u64,
    records: u64,
    keep: Option<Vec<u8>>,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl Trace {
    pub fn new() -> Self {
        Trace {
            hash: FNV_OFFSET,
            records: 0,
            keep: None,
        }
    }

    /// Also retain the canonical bytes (for golden diffs and the packet inspector).
    pub fn keeping() -> Self {
        Trace {
            keep: Some(Vec::new()),
            ..Self::new()
        }
    }

    pub fn record(&mut self, t: SimTime, kind: TraceKind, device: u32, port: u16, data: &[u8]) {
        let mut head = [0u8; 19];
        head[0..8].copy_from_slice(&t.0.to_be_bytes());
        head[8] = kind as u8;
        head[9..13].copy_from_slice(&device.to_be_bytes());
        head[13..15].copy_from_slice(&port.to_be_bytes());
        head[15..19].copy_from_slice(&(data.len() as u32).to_be_bytes());
        self.feed(&head);
        self.feed(data);
        self.records += 1;
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= b as u64;
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
        if let Some(k) = &mut self.keep {
            k.extend_from_slice(bytes);
        }
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub fn len(&self) -> u64 {
        self.records
    }

    pub fn is_empty(&self) -> bool {
        self.records == 0
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        self.keep.as_deref()
    }
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}
