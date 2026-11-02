//! Parser for `/etc/os-release` (freedesktop.org `os-release(5)`), which is a restricted
//! shell-style `KEY=VALUE` file. Only parsed — never sourced or executed.

use crate::model::OsRelease;

const MAX_BYTES: usize = 64 * 1024;
const MAX_LINE: usize = 2048;

pub fn parse_os_release(text: &str) -> OsRelease {
    let mut r = OsRelease::default();
    let text = if text.len() > MAX_BYTES {
        &text[..floor_char_boundary(text, MAX_BYTES)]
    } else {
        text
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.len() > MAX_LINE {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(raw.trim());
        match key.trim() {
            "ID" => r.id = value.to_ascii_lowercase(),
            "ID_LIKE" => {
                r.id_like = value
                    .split_whitespace()
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            }
            "NAME" => r.name = non_empty(value),
            "PRETTY_NAME" => r.pretty_name = non_empty(value),
            "VERSION" => r.version = non_empty(value),
            "VERSION_ID" => r.version_id = non_empty(value),
            "VARIANT_ID" => r.variant_id = non_empty(value),
            "BUILD_ID" => r.build_id = non_empty(value),
            _ => {}
        }
    }
    if r.id.is_empty() {
        // The spec says ID defaults to "linux".
        r.id = "linux".to_string();
    }
    r
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn unquote(v: &str) -> String {
    let bytes = v.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        let inner = &v[1..v.len() - 1];
        if bytes[0] == b'\'' {
            return inner.to_string();
        }
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some(n @ ('"' | '\\' | '$' | '`')) => out.push(n),
                    Some(n) => {
                        out.push('\\');
                        out.push(n);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const ARCH: &str = "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\nANSI_COLOR=\"38;2;23;147;209\"\nHOME_URL=\"https://archlinux.org/\"\n";
    pub const UBUNTU: &str = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nVERSION=\"24.04.1 LTS (Noble Numbat)\"\nVERSION_CODENAME=noble\nID=ubuntu\nID_LIKE=debian\n";

    #[test]
    fn arch() {
        let r = parse_os_release(ARCH);
        assert_eq!(r.id, "arch");
        assert_eq!(r.pretty_name.as_deref(), Some("Arch Linux"));
        assert_eq!(r.build_id.as_deref(), Some("rolling"));
        assert_eq!(r.version_id, None);
    }

    #[test]
    fn ubuntu_with_id_like() {
        let r = parse_os_release(UBUNTU);
        assert_eq!(r.id, "ubuntu");
        assert_eq!(r.id_like, vec!["debian"]);
        assert_eq!(r.version_id.as_deref(), Some("24.04"));
    }

    #[test]
    fn quoting_comments_and_garbage() {
        let r = parse_os_release("# c\n\nID='Fedora'\nNAME=\"Fed\\\"ora\"\nthis is not valid\n=novalue\nID_LIKE=\"rhel fedora\"\n");
        assert_eq!(r.id, "fedora");
        assert_eq!(r.name.as_deref(), Some("Fed\"ora"));
        assert_eq!(r.id_like, vec!["rhel", "fedora"]);
    }

    #[test]
    fn missing_id_defaults_to_linux() {
        assert_eq!(parse_os_release("NAME=Foo\n").id, "linux");
        assert_eq!(parse_os_release("").id, "linux");
    }

    #[test]
    fn oversized_input_is_truncated_safely() {
        let big = "ID=arch\n".to_string() + &"é".repeat(100_000);
        assert_eq!(parse_os_release(&big).id, "arch");
    }
}
