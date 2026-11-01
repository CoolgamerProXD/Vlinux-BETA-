//! Parsers for `wsl.exe` text output (already decoded by [`crate::encoding`]).
//!
//! Output is localised on non-English Windows, so parsing is structural where possible:
//! * `--list --verbose` rows are recognised by the marker column (`*` or a space), a valid
//!   distribution name and a numeric trailing VERSION — not by English column headers;
//! * state words are matched case-insensitively against the English words and otherwise
//!   reported as [`DistroState::Unknown`] (never guessed);
//! * "no distributions" / "not installed" situations are recognised by the stable
//!   `aka.ms` links and error codes embedded in the messages as well as English phrases.

use crate::model::DistroState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub name: String,
    pub state: DistroState,
    pub version: u8,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineRow {
    pub name: String,
    pub friendly_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusInfo {
    pub default_version: Option<u8>,
    pub default_distro: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionInfo {
    pub wsl: Option<String>,
    pub kernel: Option<String>,
    pub wslg: Option<String>,
}

/// WSL registered names are non-empty, at most 64 characters, `[A-Za-z0-9._-]`, and do not start
/// with `-` or `.`. Used both to validate parsed names and to gate names passed to `wsl.exe`.
pub fn valid_distro_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn parse_state(word: &str) -> DistroState {
    match word.trim().to_lowercase().as_str() {
        "running" => DistroState::Running,
        "stopped" => DistroState::Stopped,
        "installing" => DistroState::Installing,
        "uninstalling" => DistroState::Uninstalling,
        "converting" => DistroState::Converting,
        _ => DistroState::Unknown,
    }
}

/// Parse `wsl.exe --list --verbose`. Returns the rows and every non-empty line that was *not*
/// understood (headers, prose, malformed rows) so callers can warn about surprises.
pub fn parse_list_verbose(text: &str) -> (Vec<ListRow>, Vec<String>) {
    let mut rows = Vec::new();
    let mut unparsed = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        // Structural guard: table rows begin with the default-marker column ("* " or "  ").
        // Prose messages start at column 0.
        let first = line.chars().next().unwrap_or('x');
        let marker_col = first == '*' || first == ' ';
        let is_default = first == '*';
        let body = line.trim_start().trim_start_matches('*').trim();
        let toks: Vec<&str> = body.split_whitespace().collect();
        let mut parsed = None;
        if marker_col && toks.len() >= 3 && toks.len() <= 6 {
            let name = toks[0];
            let version: Option<u8> = toks[toks.len() - 1].parse().ok().filter(|v| *v == 1 || *v == 2);
            if let (true, Some(version)) = (valid_distro_name(name), version) {
                let state = parse_state(&toks[1..toks.len() - 1].join(" "));
                parsed = Some(ListRow { name: name.to_string(), state, version, is_default });
            }
        }
        match parsed {
            Some(r) => rows.push(r),
            None => unparsed.push(line.trim().to_string()),
        }
    }
    (rows, unparsed)
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let idx = line.find([':', '：'])?;
    let (k, rest) = line.split_at(idx);
    let v = rest.trim_start_matches([':', '：']);
    Some((k.trim(), v.trim()))
}

/// Parse `wsl.exe --status`.
pub fn parse_status(text: &str) -> StatusInfo {
    let mut info = StatusInfo::default();
    let mut numeric_fallback: Option<u8> = None;
    for line in text.lines() {
        let Some((k, v)) = split_kv(line) else { continue };
        let kl = k.to_lowercase();
        if kl.contains("default") && kl.contains("distribution") {
            if valid_distro_name(v) {
                info.default_distro = Some(v.to_string());
            }
        } else if kl.contains("default") && kl.contains("version") {
            info.default_version = v.parse::<u8>().ok().filter(|n| *n == 1 || *n == 2);
        } else if numeric_fallback.is_none() && !kl.contains("kernel") {
            // Localised label: accept the first "label: 1|2" line as the default version.
            numeric_fallback = v.parse::<u8>().ok().filter(|n| *n == 1 || *n == 2);
        }
    }
    if info.default_version.is_none() {
        info.default_version = numeric_fallback;
    }
    info
}

fn first_token(v: &str) -> Option<String> {
    let t = v.split_whitespace().next()?;
    if t.chars().next()?.is_ascii_digit() {
        Some(t.to_string())
    } else {
        None
    }
}

/// Parse `wsl.exe --version` (WSL from the Store/MSI; absent on very old inbox WSL).
pub fn parse_version(text: &str) -> VersionInfo {
    let mut info = VersionInfo::default();
    let mut ordered: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = split_kv(line) else { continue };
        let Some(ver) = first_token(v) else { continue };
        ordered.push(ver.clone());
        let kl = k.to_lowercase();
        if kl.contains("wslg") {
            info.wslg.get_or_insert(ver);
        } else if kl.contains("kernel") {
            info.kernel.get_or_insert(ver);
        } else if kl.starts_with("wsl") {
            info.wsl.get_or_insert(ver);
        }
    }
    if info.wsl.is_none() && info.kernel.is_none() && info.wslg.is_none() {
        // Localised labels: WSL prints WSL, Kernel, WSLg ... in that order.
        info.wsl = ordered.first().cloned();
        info.kernel = ordered.get(1).cloned();
        info.wslg = ordered.get(2).cloned();
    }
    info
}

/// Parse `wsl.exe --list --online`: rows are `NAME<2+ spaces>FRIENDLY NAME`.
pub fn parse_online_list(text: &str) -> Vec<OnlineRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with(' ') {
            continue;
        }
        // Two or more spaces separate the columns; prose lines use single spaces.
        let Some(gap) = line.find("  ") else { continue };
        let name = &line[..gap];
        let friendly = line[gap..].trim();
        if friendly.is_empty() || !valid_distro_name(name) {
            continue;
        }
        // Skip the (possibly localised) header row: an all-caps word without digits.
        if name.len() >= 2 && name.chars().all(|c| c.is_ascii_uppercase()) {
            continue;
        }
        rows.push(OnlineRow { name: name.to_string(), friendly_name: friendly.to_string() });
    }
    rows
}

/// Markers inside WSL's failure messages that do not depend on the display language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// WSL (or its optional components) are not installed / enabled.
    NotInstalled,
    /// WSL works but has no distributions registered.
    NoDistributions,
    /// A recognised actionable error code.
    Code { code: &'static str, hint: &'static str },
}

const CODES: &[(&str, &str)] = &[
    (
        "0x80370102",
        "Virtualisation is not available. Enable hardware virtualisation (VT-x / AMD-V / SVM) in the firmware settings and make sure the 'Virtual Machine Platform' Windows feature is on.",
    ),
    (
        "0x80370114",
        "A required Windows feature is not installed. Enable 'Virtual Machine Platform' (and 'Windows Subsystem for Linux') in Windows Features, then restart.",
    ),
    (
        "0x8007019e",
        "The 'Windows Subsystem for Linux' optional component is not enabled. Enable it in Windows Features (or run 'wsl.exe --install'), then restart.",
    ),
    (
        "0x800701bc",
        "WSL 2 needs a kernel update. Run 'wsl.exe --update' from a normal terminal.",
    ),
];

pub fn classify_failure(text: &str) -> Option<Failure> {
    let lc = text.to_lowercase();
    for &(code, hint) in CODES {
        if lc.contains(code) {
            return Some(Failure::Code { code, hint });
        }
    }
    // The 'not installed' stub message links to aka.ms/wslinstall in every language.
    if lc.contains("aka.ms/wslinstall") || lc.contains("is not installed") && lc.contains("wsl.exe --install") {
        return Some(Failure::NotInstalled);
    }
    // The 'no distributions' message links to the Store page / suggests --list --online.
    if lc.contains("aka.ms/wslstore")
        || lc.contains("has no installed distributions")
        || lc.contains("--list --online") && lc.contains("--install")
    {
        return Some(Failure::NoDistributions);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "  NAME              STATE           VERSION\n* archlinux         Running         2\n  Ubuntu-24.04      Stopped         2\n  Debian            Stopped         1\n";

    #[test]
    fn list_rows_state_default_and_version() {
        let (rows, unparsed) = parse_list_verbose(LIST);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ListRow { name: "archlinux".into(), state: DistroState::Running, version: 2, is_default: true });
        assert_eq!(rows[1].state, DistroState::Stopped);
        assert!(!rows[1].is_default);
        assert_eq!(rows[2].version, 1);
        assert_eq!(unparsed, vec!["NAME              STATE           VERSION"]);
    }

    #[test]
    fn localised_state_words_with_spaces_are_unknown_not_guessed() {
        let (rows, _) = parse_list_verbose("  NOM   ÉTAT   VERSION\n* Ubuntu   En cours d'exécution   2\n  Debian   Arrêté   2\n");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_default);
        assert_eq!(rows[0].state, DistroState::Unknown);
        assert_eq!(rows[1].name, "Debian");
    }

    #[test]
    fn prose_is_never_mistaken_for_a_row() {
        let text = "Windows Subsystem for Linux has no installed distributions.\nUse wsl.exe --set-version 2\nDistributions can be installed from https://aka.ms/wslstore\n";
        let (rows, unparsed) = parse_list_verbose(text);
        assert!(rows.is_empty());
        assert_eq!(unparsed.len(), 3);
    }

    #[test]
    fn malformed_rows_are_reported_not_fatal() {
        let (rows, unparsed) = parse_list_verbose("* Ubuntu Running\n  Bad/Name Stopped 2\n  Good Stopped 2\n  Weird Stopped 7\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Good");
        assert_eq!(unparsed.len(), 3);
    }

    #[test]
    fn empty_and_garbage_input() {
        assert_eq!(parse_list_verbose("").0.len(), 0);
        assert_eq!(parse_list_verbose("\n\n  \n").0.len(), 0);
        assert_eq!(parse_list_verbose("\u{1F980}\u{1F980}\u{1F980}").0.len(), 0);
    }

    #[test]
    fn status_english_and_localised() {
        let s = parse_status("Default Distribution: Ubuntu\nDefault Version: 2\n");
        assert_eq!(s, StatusInfo { default_version: Some(2), default_distro: Some("Ubuntu".into()) });
        let de = parse_status("Standarddistribution: Ubuntu\nStandardversion: 2\n");
        assert_eq!(de.default_version, Some(2));
        assert_eq!(parse_status("Default Version: 2\n").default_distro, None);
        assert_eq!(parse_status("nonsense").default_version, None);
    }

    #[test]
    fn version_output() {
        let v = parse_version("WSL version: 2.4.13.0\nKernel version: 5.15.167.4-1\nWSLg version: 1.0.65\nMSRDC version: 1.2.5716\nWindows version: 10.0.26100.1\n");
        assert_eq!(v, VersionInfo { wsl: Some("2.4.13.0".into()), kernel: Some("5.15.167.4-1".into()), wslg: Some("1.0.65".into()) });
        let de = parse_version("WSL-Version: 2.1.5.0\nKernelversion: 5.15.146.1\nWSLg-Version: 1.0.60\n");
        assert_eq!(de.wsl.as_deref(), Some("2.1.5.0"));
        assert_eq!(de.kernel.as_deref(), Some("5.15.146.1"));
        assert_eq!(parse_version("garbage"), VersionInfo::default());
    }

    #[test]
    fn online_list() {
        let text = "The following is a list of valid distributions that can be installed.\nInstall using 'wsl.exe --install <Distro>'.\n\nNAME                            FRIENDLY NAME\nUbuntu                          Ubuntu\nDebian                          Debian GNU/Linux\nkali-linux                      Kali Linux Rolling\nFedoraLinux-42                  Fedora Linux 42\narchlinux                       Arch Linux\n";
        let rows = parse_online_list(text);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Ubuntu", "Debian", "kali-linux", "FedoraLinux-42", "archlinux"]);
        assert_eq!(rows[1].friendly_name, "Debian GNU/Linux");
    }

    #[test]
    fn failure_classification() {
        assert_eq!(classify_failure("The Windows Subsystem for Linux is not installed. ... https://aka.ms/wslinstall"), Some(Failure::NotInstalled));
        assert_eq!(classify_failure("Windows Subsystem for Linux has no installed distributions.\nhttps://aka.ms/wslstore"), Some(Failure::NoDistributions));
        assert!(matches!(classify_failure("Error: 0x80370102 The virtual machine could not be started"), Some(Failure::Code { code: "0x80370102", .. })));
        assert!(matches!(classify_failure("WSL2 ERROR: 0X8007019E"), Some(Failure::Code { code: "0x8007019e", .. })));
        assert_eq!(classify_failure("something else entirely"), None);
    }

    #[test]
    fn names() {
        assert!(valid_distro_name("Ubuntu-24.04"));
        assert!(valid_distro_name("archlinux"));
        assert!(!valid_distro_name(""));
        assert!(!valid_distro_name("-rf"));
        assert!(!valid_distro_name("a b"));
        assert!(!valid_distro_name("a;b"));
        assert!(!valid_distro_name("../x"));
        assert!(!valid_distro_name(&"a".repeat(65)));
    }
}
