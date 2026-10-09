//! Time-based one-time passwords (RFC 6238 / RFC 4226) with HMAC-SHA1, recovery
//! codes, and the setup QR code drawn on a map.

use hmac::{Hmac, Mac};
use pumbo_common::map::{BLACK, Canvas, SIZE, SNOW, color};
use sha1::Sha1;
use sha2::{Digest, Sha256};

pub const PERIOD: u64 = 30;
pub const DIGITS: u32 = 6;

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &b in data {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            let idx = ((buffer >> (bits - 5)) & 31) as usize;
            out.push(char::from(B32.get(idx).copied().unwrap_or(b'A')));
            bits -= 5;
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 31) as usize;
        out.push(char::from(B32.get(idx).copied().unwrap_or(b'A')));
    }
    out
}

pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for c in text.chars() {
        if c == '=' || c == ' ' || c == '-' {
            continue;
        }
        let c = c.to_ascii_uppercase();
        let val = B32.iter().position(|&b| char::from(b) == c)? as u32;
        buffer = (buffer << 5) | val;
        bits += 5;
        if bits >= 8 {
            out.push(((buffer >> (bits - 8)) & 0xFF) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// HOTP value (RFC 4226) for a counter.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> Option<u32> {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).ok()?;
    mac.update(&counter.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash.get(19)? & 0x0F);
    let bytes = hash.get(offset..offset + 4)?;
    let code = (u32::from(bytes.first()? & 0x7F) << 24)
        | (u32::from(*bytes.get(1)?) << 16)
        | (u32::from(*bytes.get(2)?) << 8)
        | u32::from(*bytes.get(3)?);
    Some(code % 10u32.pow(digits))
}

pub fn totp_at(secret: &[u8], unix_seconds: u64, digits: u32) -> Option<u32> {
    hotp(secret, unix_seconds / PERIOD, digits)
}

/// Checks a 6-digit code within ±1 time step. Returns the matched step so the
/// caller can refuse to accept the same code twice.
pub fn verify(secret: &[u8], code: &str, unix_seconds: u64, last_used_step: Option<u64>) -> Option<u64> {
    let code = code.trim().replace(' ', "");
    if code.len() != DIGITS as usize || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let now = unix_seconds / PERIOD;
    for step in [now, now.saturating_sub(1), now + 1] {
        if last_used_step.is_some_and(|l| step <= l) {
            continue;
        }
        if hotp(secret, step, DIGITS) == Some(want) {
            return Some(step);
        }
    }
    None
}

/// New random 160-bit secret.
pub fn new_secret() -> Vec<u8> {
    let mut b = vec![0u8; 20];
    pumbo_common::random::random_bytes(&mut b);
    b
}

/// `otpauth://` link understood by authenticator apps.
pub fn otpauth_url(issuer: &str, account: &str, secret_b32: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&digits={}&period={}",
        url_encode(issuer),
        url_encode(account),
        secret_b32,
        url_encode(issuer),
        DIGITS,
        PERIOD
    )
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Generates `count` recovery codes in the form `xxxx-xxxx`.
pub fn recovery_codes(count: u32) -> Vec<String> {
    const ALPHA: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..count)
        .map(|_| {
            let mut raw = [0u8; 8];
            pumbo_common::random::random_bytes(&mut raw);
            let chars: String = raw
                .iter()
                .map(|b| char::from(ALPHA.get(usize::from(*b) % ALPHA.len()).copied().unwrap_or(b'a')))
                .collect();
            let (a, b) = chars.split_at(4);
            format!("{a}-{b}")
        })
        .collect()
}

/// Recovery codes are stored hashed.
pub fn hash_recovery(code: &str) -> String {
    let norm: String = code.trim().to_lowercase().chars().filter(|c| *c != '-' && *c != ' ').collect();
    pumbo_common::util::hex(&Sha256::digest(norm.as_bytes()))
}

/// Renders text (an `otpauth://` link) as a QR code centred on a white map.
pub fn qr_image(text: &str) -> Option<Vec<u8>> {
    use qrcodegen::{QrCode, QrCodeEcc};
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let size = qr.size();
    // A quiet zone of 2 modules on each side.
    let scale = (SIZE as i32 / (size + 4)).max(1);
    if scale * size > SIZE as i32 {
        return None;
    }
    let offset = (SIZE as i32 - scale * size) / 2;
    let mut c = Canvas::new(color(SNOW, 2));
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                c.fill_rect(offset + x * scale, offset + y * scale, scale, scale, color(BLACK, 2));
            }
        }
    }
    Some(c.px)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vectors from RFC 6238 appendix B (SHA1, 8 digits).
    #[test]
    fn rfc6238_vectors() {
        let secret = b"12345678901234567890";
        let cases = [
            (59u64, 94_287_082u32),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
            (20_000_000_000, 65_353_130),
        ];
        for (t, want) in cases {
            assert_eq!(totp_at(secret, t, 8), Some(want), "t={t}");
        }
    }

    /// RFC 4226 appendix D.
    #[test]
    fn rfc4226_vectors() {
        let secret = b"12345678901234567890";
        let want = [755_224u32, 287_082, 359_152, 969_429, 338_314, 254_676, 287_922, 162_583, 399_871, 520_489];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(hotp(secret, i as u64, 6), Some(*w));
        }
    }

    #[test]
    fn base32_roundtrip() {
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32_decode("MZXW6YTBOI").unwrap(), b"foobar");
        assert_eq!(base32_decode("mzxw 6ytb oi====").unwrap(), b"foobar");
        assert!(base32_decode("0189").is_none());
        let s = new_secret();
        assert_eq!(base32_decode(&base32_encode(&s)).unwrap(), s);
    }

    #[test]
    fn verify_window_and_replay() {
        let secret = b"12345678901234567890";
        let t = 1_234_567_890;
        let code = format!("{:06}", totp_at(secret, t, 6).unwrap());
        let step = verify(secret, &code, t, None).unwrap();
        assert_eq!(step, t / PERIOD);
        // previous step still accepted
        assert!(verify(secret, &code, t + PERIOD, None).is_some());
        // two steps later it is not
        assert!(verify(secret, &code, t + 3 * PERIOD, None).is_none());
        // the same code cannot be used twice
        assert!(verify(secret, &code, t, Some(step)).is_none());
        assert!(verify(secret, "12345", t, None).is_none());
        assert!(verify(secret, "abcdef", t, None).is_none());
    }

    #[test]
    fn url_and_recovery() {
        let url = otpauth_url("My Server", "Steve", "ABC");
        assert_eq!(url, "otpauth://totp/My%20Server:Steve?secret=ABC&issuer=My%20Server&digits=6&period=30");
        let codes = recovery_codes(8);
        assert_eq!(codes.len(), 8);
        assert!(codes.iter().all(|c| c.len() == 9 && c.as_bytes()[4] == b'-'));
        assert_eq!(hash_recovery("ABCD-efgh"), hash_recovery("abcdefgh"));
    }

    #[test]
    fn qr_fits_on_a_map() {
        let url = otpauth_url("Pumbo", "Steve_Minecraft1", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP");
        let img = qr_image(&url).unwrap();
        assert_eq!(img.len(), pumbo_common::map::PIXELS);
        let dark = img.iter().filter(|p| **p == color(BLACK, 2)).count();
        assert!(dark > 1000, "{dark}");
    }
}
