/// RFC 1071 internet checksum over `data`. Odd trailing byte is padded with zero.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_checksum_rfc1071_vectors() {
        // RFC 1071 section 3 example: words 0001 f203 f4f5 f6f7 sum to ddf2 (ones' complement).
        let d = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(internet_checksum(&d), !0xddf2);
        // A buffer containing its own checksum verifies to zero.
        let mut v = d.to_vec();
        v.extend_from_slice(&internet_checksum(&d).to_be_bytes());
        assert_eq!(internet_checksum(&v), 0);
        // Known IPv4 header (checksum field zeroed) -> 0xb861.
        let h = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0x00, 0x00, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        assert_eq!(internet_checksum(&h), 0xb861);
    }

    #[test]
    fn odd_length_pads_zero() {
        assert_eq!(internet_checksum(&[0xff]), !0xff00);
    }
}
