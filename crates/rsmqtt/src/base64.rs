//! Standard base64 (RFC 4648, `+` `/`, padded), for SCRAM and for the
//! dyn_blob items of the WinCC OA driver (rsmqtt-wincc).

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Appends `data` in base64, with padding.
pub fn encode_into(data: &[u8], out: &mut Vec<u8>) {
    out.reserve(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63]);
        out.push(B64[(n >> 12) as usize & 63]);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] } else { b'=' });
    }
}

/// `data` in base64, with padding.
pub fn encode(data: &[u8]) -> String {
    let mut out = Vec::new();
    encode_into(data, &mut out);
    // only characters of B64 and '='
    String::from_utf8(out).unwrap_or_default()
}

/// Decodes base64; the padding may be left out, white space is not allowed,
/// and the unused bits of the last character must be 0.
pub fn decode(text: &[u8]) -> Result<Vec<u8>, String> {
    let body = text.strip_suffix(b"==").or_else(|| text.strip_suffix(b"=")).unwrap_or(text);
    if text.len() != body.len() && !text.len().is_multiple_of(4) {
        return Err("invalid base64 padding".into());
    }
    if body.len() % 4 == 1 {
        return Err("invalid base64 length".into());
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in body {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("invalid base64 character {:?}", c as char)),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if acc & ((1 << bits) - 1) != 0 {
        return Err("invalid base64 (trailing bits)".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for (raw, enc) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (b"\x00\xff\xfe", "AP/+"),
        ] {
            assert_eq!(encode(raw), enc);
            assert_eq!(decode(enc.as_bytes()).unwrap(), raw);
        }
        assert_eq!(decode(b"Zg").unwrap(), b"f");
        assert_eq!(decode(b"Zm8").unwrap(), b"fo");
        for bad in ["Z", "Zg=", "Zh==", "Zm9v Yg", "Zm9v\n", "Zm-v", "=", "Zg===", "Z==="] {
            assert!(decode(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
