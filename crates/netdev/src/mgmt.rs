//! Management-plane state shared by routers and switches: enable passwords, console and vty
//! lines, the login banner, and the saved (startup) configuration.
//!
//! Secrets are never stored or printed in a vendor's formats. A secret is kept as
//! `<salt>$<digest>` and written `wb1$<salt>$<digest>`; a plain password is kept as typed and
//! written `wb0$<hex>` while `service password-encryption` is on. Both are teaching aids, not
//! security: the digest is a few rounds of a non-cryptographic hash and the `wb0$` form is a
//! reversible xor. The two encoded forms are recognised by shape, not by a keyword in the
//! command, so an ordinary password is never mistaken for one (a password that happens to look
//! exactly like an encoded form is read as that form).

/// One `line` block: `line console 0` or `line vty 0 4`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub password: Option<String>,
    pub login: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mgmt {
    /// `<salt>$<digest>` of the enable secret.
    pub enable_secret: Option<String>,
    /// The enable password in the clear; written as `wb0` while `encrypt_passwords` is on.
    pub enable_password: Option<String>,
    /// `service password-encryption`.
    pub encrypt_passwords: bool,
    pub banner_motd: Option<String>,
    pub console: Line,
    pub vty: Line,
    /// The range given to `line vty`, kept so the config prints what was typed.
    pub vty_range: (u8, u8),
}

impl Default for Mgmt {
    fn default() -> Self {
        Mgmt {
            enable_secret: None,
            enable_password: None,
            encrypt_passwords: false,
            banner_motd: None,
            console: Line::default(),
            vty: Line::default(),
            vty_range: (0, 4),
        }
    }
}

impl Mgmt {
    /// Whether `enable` asks for a password.
    pub fn enable_protected(&self) -> bool {
        self.enable_secret.is_some() || self.enable_password.is_some()
    }

    /// Check a typed enable password: the secret if there is one, otherwise the password.
    pub fn check_enable(&self, attempt: &str) -> bool {
        match (&self.enable_secret, &self.enable_password) {
            (Some(s), _) => check_secret(s, attempt),
            (None, Some(p)) => p == attempt,
            (None, None) => true,
        }
    }

    /// Whether a new console session must log in.
    pub fn console_needs_login(&self) -> bool {
        self.console.login && self.console.password.is_some()
    }
}

fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Hash `secret` for storage. The salt comes from `device` so the result is the same every run
/// (simulations must be reproducible) yet differs between devices.
pub fn hash_secret(device: &str, secret: &str) -> String {
    digest(fnv(0, device.as_bytes()) as u32, secret)
}

fn digest(salt: u32, secret: &str) -> String {
    let mut h = fnv(u64::from(salt), secret.as_bytes());
    for _ in 0..1000 {
        h = fnv(h, secret.as_bytes());
    }
    format!("{salt:08x}${h:016x}")
}

/// Whether `stored` has the shape of [`hash_secret`]'s output.
pub fn is_hashed(stored: &str) -> bool {
    match stored.split_once('$') {
        Some((salt, d)) => {
            salt.len() == 8
                && d.len() == 16
                && salt.bytes().chain(d.bytes()).all(|b| b.is_ascii_hexdigit())
        }
        None => false,
    }
}

pub fn check_secret(stored: &str, attempt: &str) -> bool {
    let Some((salt, _)) = stored.split_once('$') else {
        return false;
    };
    u32::from_str_radix(salt, 16).is_ok_and(|s| digest(s, attempt) == stored)
}

const KEY: &[u8] = b"wirebench";

/// The reversible `wb0` form of a plain password, as hex.
pub fn obscure(plain: &str) -> String {
    plain
        .bytes()
        .enumerate()
        .map(|(i, b)| format!("{:02x}", b ^ KEY[i % KEY.len()]))
        .collect()
}

/// Undo [`obscure`]. `None` if `hex` is not what it would have produced.
pub fn reveal(hex: &str) -> Option<String> {
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len() / 2)
        .map(|i| {
            u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
                .ok()
                .map(|b| b ^ KEY[i % KEY.len()])
        })
        .collect();
    String::from_utf8(bytes?)
        .ok()
        .filter(|s| !s.is_empty() && !s.contains(char::is_whitespace))
}

/// A password as it appears in a config line: plain, or `wb0$<hex>` under password encryption.
pub fn password_text(plain: &str, encrypt: bool) -> String {
    if encrypt {
        format!("wb0${}", obscure(plain))
    } else {
        plain.to_string()
    }
}

/// The plain password for a word typed or replayed: `wb0$<hex>` decodes, anything else is taken
/// as typed.
pub fn plain_password(word: &str) -> String {
    word.strip_prefix("wb0$")
        .and_then(reveal)
        .unwrap_or_else(|| word.to_string())
}

/// The stored form for an enable secret typed or replayed: `wb1$<salt>$<digest>` is kept as
/// is, anything else is hashed (salted with the device name).
pub fn stored_secret(device: &str, word: &str) -> String {
    match word.strip_prefix("wb1$") {
        Some(h) if is_hashed(h) => h.to_string(),
        _ => hash_secret(device, word),
    }
}

/// A delimiter that does not occur in `text`, for printing `banner motd`.
pub fn banner_delimiter(text: &str) -> char {
    ['#', '$', '%', '^', '~', '@', '&']
        .into_iter()
        .find(|c| !text.contains(*c))
        .unwrap_or('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_verify_and_are_stable() {
        let h = hash_secret("r1", "cisco123");
        assert!(is_hashed(&h), "{h}");
        assert!(check_secret(&h, "cisco123"));
        assert!(!check_secret(&h, "Cisco123"));
        assert_eq!(h, hash_secret("r1", "cisco123"), "same inputs, same output");
        assert_ne!(h, hash_secret("r2", "cisco123"), "salted per device");
        assert!(!h.contains("cisco123"));
    }

    #[test]
    fn hashed_shape_is_strict() {
        for bad in [
            "",
            "plain",
            "abc$def",
            "0123456$0123456789abcdef",
            "0123456g$0123456789abcdef",
        ] {
            assert!(!is_hashed(bad), "{bad}");
        }
        assert!(!check_secret("plain", "plain"));
    }

    #[test]
    fn obscured_passwords_round_trip() {
        for p in ["a", "letmein", "p@ss-w0rd!", "ünï"] {
            let o = obscure(p);
            assert!(!o.contains(p) || p.len() == 1);
            assert_eq!(reveal(&o).as_deref(), Some(p));
        }
        assert_eq!(reveal(""), None);
        assert_eq!(reveal("abc"), None);
        assert_eq!(reveal("zz"), None);
    }

    #[test]
    fn encoded_forms_are_recognised_by_shape_and_everything_else_is_a_password() {
        let h = hash_secret("r", "pw");
        assert_eq!(stored_secret("r", &format!("wb1${h}")), h);
        assert_eq!(stored_secret("r", "pw"), h, "same device, same secret");
        for odd in ["w", "wb", "wb1", "wb1$", "wb1$nope", "wb0$zz"] {
            assert!(check_secret(&stored_secret("r", odd), odd), "{odd}");
        }
        assert_eq!(plain_password(&password_text("hunter2", true)), "hunter2");
        for odd in ["w", "wb0", "wb0$", "wb0$zz", "wb0$00"] {
            if odd != "wb0$00" {
                assert_eq!(plain_password(odd), odd, "{odd}");
            }
        }
    }

    #[test]
    fn enable_prefers_the_secret() {
        let mut m = Mgmt::default();
        assert!(!m.enable_protected() && m.check_enable(""));
        m.enable_password = Some("pw".into());
        assert!(m.check_enable("pw") && !m.check_enable("x"));
        m.enable_secret = Some(hash_secret("r", "sec"));
        assert!(m.check_enable("sec") && !m.check_enable("pw"));
    }

    #[test]
    fn banner_delimiter_avoids_the_text() {
        assert_eq!(banner_delimiter("hello"), '#');
        assert_eq!(banner_delimiter("a # b"), '$');
    }
}
