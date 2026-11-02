//! The parts of a distribution's `/etc/wsl.conf` that decide where Windows drives are mounted.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslConf {
    /// `[automount] enabled` (default true). When false, Windows drives are *not* mounted and a
    /// shared folder cannot be linked.
    pub automount_enabled: bool,
    /// `[automount] root` (default `/mnt`), without a trailing slash.
    pub mount_root: String,
}

impl Default for WslConf {
    fn default() -> Self {
        Self { automount_enabled: true, mount_root: "/mnt".into() }
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

pub fn parse_wsl_conf(text: &str) -> WslConf {
    let mut conf = WslConf::default();
    let mut section = String::new();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            section = rest.trim_end_matches(']').trim().to_ascii_lowercase();
            continue;
        }
        if section != "automount" {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim().trim_matches('"').trim();
        match key.as_str() {
            "enabled" => match value.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => conf.automount_enabled = true,
                "false" | "no" | "0" => conf.automount_enabled = false,
                _ => {}
            },
            "root" => {
                if value.starts_with('/') && !value.contains(['\0', '\\']) {
                    let trimmed = value.trim_end_matches('/');
                    conf.mount_root = if trimmed.is_empty() { "/".into() } else { trimmed.to_string() };
                }
            }
            _ => {}
        }
    }
    conf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        assert_eq!(parse_wsl_conf(""), WslConf::default());
        assert_eq!(WslConf::default().mount_root, "/mnt");
    }

    #[test]
    fn custom_root_and_disabled_automount() {
        let c = parse_wsl_conf("[automount]\nroot = /windir/\nenabled=false # off\n[interop]\nenabled = true\n");
        assert_eq!(c.mount_root, "/windir");
        assert!(!c.automount_enabled);
    }

    #[test]
    fn other_sections_and_junk_are_ignored() {
        let c = parse_wsl_conf("[user]\ndefault=me\n[network]\nroot = /x\n[automount]\nroot = relative\nroot = /ok\n[boot]\nsystemd=true\n");
        assert_eq!(c.mount_root, "/ok");
        assert!(c.automount_enabled);
        // Mixed case and quotes.
        assert_eq!(parse_wsl_conf("[AutoMount]\nRoot = \"/drives\"").mount_root, "/drives");
    }
}
