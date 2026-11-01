//! Decoding of `wsl.exe` output.
//!
//! `wsl.exe` historically writes UTF-16LE (usually without a BOM) when its output is
//! redirected, but some messages are UTF-8, and newer builds honour `WSL_UTF8=1`. Output
//! may also contain stray NUL characters. [`decode_wsl_output`] copes with all of these and
//! never panics on arbitrary bytes.

/// Decode raw process output into clean text with `\n` line endings, no NULs and no BOM.
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, false)
    } else if looks_like_utf16(bytes, true) {
        utf16(bytes, true)
    } else if looks_like_utf16(bytes, false) {
        utf16(bytes, false)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                // Not UTF-8. Non-ASCII UTF-16 text (e.g. localised CJK) has no zero bytes, so try
                // a strict UTF-16LE decode before giving up and substituting characters.
                if bytes.len() % 2 == 0 {
                    if let Some(s) = strict_utf16(bytes) {
                        return normalise(&s);
                    }
                }
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
    };
    normalise(&text)
}

/// ASCII-heavy UTF-16 has a zero in (almost) every second byte.
fn looks_like_utf16(bytes: &[u8], little_endian: bool) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let pairs = bytes.len() / 2;
    let (zero_idx, other_idx) = if little_endian { (1, 0) } else { (0, 1) };
    let mut zeros_in_high = 0usize;
    let mut zeros_in_low = 0usize;
    for p in 0..pairs {
        if bytes[p * 2 + zero_idx] == 0 {
            zeros_in_high += 1;
        }
        if bytes[p * 2 + other_idx] == 0 {
            zeros_in_low += 1;
        }
    }
    // Most high bytes are zero (ASCII) and the low bytes are not.
    zeros_in_high * 10 >= pairs * 6 && zeros_in_low * 10 < pairs
}

fn utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| if little_endian { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

fn strict_utf16(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let s = String::from_utf16(&units).ok()?;
    let sane = s
        .chars()
        .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t' || c == '\0');
    if sane {
        Some(s)
    } else {
        None
    }
}

fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\0' | '\u{FEFF}' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }
    fn be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(decode_wsl_output(b""), "");
    }

    #[test]
    fn utf16le_without_bom() {
        assert_eq!(decode_wsl_output(&le("  NAME  STATE\r\n* Ubuntu  Running  2\r\n")), "  NAME  STATE\n* Ubuntu  Running  2\n");
    }

    #[test]
    fn utf16le_with_bom() {
        let mut b = vec![0xFF, 0xFE];
        b.extend(le("Default Version: 2\r\n"));
        assert_eq!(decode_wsl_output(&b), "Default Version: 2\n");
    }

    #[test]
    fn utf16be_with_and_without_bom() {
        let mut b = vec![0xFE, 0xFF];
        b.extend(be("hello\n"));
        assert_eq!(decode_wsl_output(&b), "hello\n");
        assert_eq!(decode_wsl_output(&be("hello world\n")), "hello world\n");
    }

    #[test]
    fn plain_utf8_and_utf8_bom() {
        assert_eq!(decode_wsl_output("Ubuntu — Läuft\r\n".as_bytes()), "Ubuntu — Läuft\n");
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice(b"ok");
        assert_eq!(decode_wsl_output(&b), "ok");
    }

    #[test]
    fn stray_nuls_are_removed() {
        assert_eq!(decode_wsl_output(b"abc\0def\0"), "abcdef");
    }

    #[test]
    fn non_ascii_utf16_localised_text() {
        let s = "已安装的分发\r\n";
        assert_eq!(decode_wsl_output(&le(s)), "已安装的分发\n");
    }

    #[test]
    fn utf16_with_accents() {
        assert_eq!(decode_wsl_output(&le("Standardversion: 2\r\nLäuft\r\n")), "Standardversion: 2\nLäuft\n");
    }

    #[test]
    fn lone_cr_becomes_newline_for_progress_output() {
        assert_eq!(decode_wsl_output(b"Installing: 10%\rInstalling: 20%\r"), "Installing: 10%\nInstalling: 20%\n");
    }

    #[test]
    fn garbage_never_panics() {
        let junk: Vec<u8> = (0..=255u8).collect();
        let _ = decode_wsl_output(&junk);
        let _ = decode_wsl_output(&[0xFF]);
        let _ = decode_wsl_output(&[0xFF, 0xFE, 0x41]);
        let _ = decode_wsl_output(&[0xD8, 0x00, 0x00, 0x41]);
    }
}
