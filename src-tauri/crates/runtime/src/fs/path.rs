//! Path and name validation. Pure string logic.
//!
//! Locations that come from the UI are validated here *before* anything touches a filesystem:
//! Linux paths must be absolute and free of `..` (the UI navigates "up" explicitly), names used
//! for creating or renaming must be valid on both sides (all access goes through Windows file
//! APIs, which are stricter than Linux).

use super::error::{FsError, FsErrorKind, FsResult};

pub const MAX_PATH: usize = 4096;
pub const MAX_NAME: usize = 255;

/// Characters that Windows file APIs cannot address (`:` would also select an alternate data
/// stream, `*`/`?` are wildcards).
pub const WINDOWS_FORBIDDEN: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

fn invalid_path(msg: impl Into<String>) -> FsError {
    FsError::new(FsErrorKind::InvalidPath, msg)
}

/// Split an absolute Linux path into components. Empty parts and `.` are dropped; `..`, control
/// characters (including NUL), backslashes and over-long paths are refused.
pub fn linux_components(path: &str) -> FsResult<Vec<String>> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(invalid_path("A Linux path must start with '/'."));
    }
    if path.len() > MAX_PATH {
        return Err(invalid_path("That path is too long."));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(invalid_path("The path contains control characters."));
    }
    if path.contains('\\') {
        return Err(invalid_path("Backslashes are not supported in Linux paths here."));
    }
    let mut out = Vec::new();
    for c in path.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            return Err(invalid_path("'..' is not allowed in a path. Use the Up button instead."));
        }
        if c.len() > MAX_NAME {
            return Err(FsError::new(FsErrorKind::InvalidName, "A name in that path is too long."));
        }
        out.push(c.to_string());
    }
    Ok(out)
}

pub fn linux_join(comps: &[String]) -> String {
    if comps.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", comps.join("/"))
    }
}

/// Canonical text form (`/a/b`) of a valid Linux path.
pub fn normalize_linux(path: &str) -> FsResult<String> {
    Ok(linux_join(&linux_components(path)?))
}

pub fn linux_parent(path: &str) -> Option<String> {
    let comps = linux_components(path).ok()?;
    if comps.is_empty() {
        None
    } else {
        Some(linux_join(&comps[..comps.len() - 1]))
    }
}

fn is_reserved_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.chars().nth(3).is_some_and(|c| ('1'..='9').contains(&c)))
}

/// A name for something *new* (create, rename, paste). Valid on Linux and addressable from
/// Windows.
pub fn check_name(name: &str) -> FsResult<()> {
    let bad = |m: &str| Err(FsError::new(FsErrorKind::InvalidName, m.to_string()));
    if name.is_empty() {
        return bad("Please enter a name.");
    }
    if name == "." || name == ".." {
        return bad("\".\" and \"..\" are not valid names.");
    }
    if name.len() > MAX_NAME {
        return bad("That name is too long (255 bytes at most).");
    }
    if name.contains('/') || name.contains('\\') {
        return bad("A name cannot contain a slash or a backslash.");
    }
    if name.chars().any(|c| c.is_control()) {
        return bad("A name cannot contain control characters.");
    }
    if let Some(c) = name.chars().find(|c| WINDOWS_FORBIDDEN.contains(c)) {
        return bad(&format!(
            "A name cannot contain \"{c}\": files are reached through Windows file APIs, which do not allow < > : \" | ? *. Use a terminal for such names."
        ));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return bad("A name cannot end with a space or a dot.");
    }
    if name.starts_with(' ') {
        return bad("A name cannot start with a space.");
    }
    if is_reserved_device(name) {
        return bad("That is a reserved Windows device name (CON, PRN, AUX, NUL, COM1…, LPT1…).");
    }
    Ok(())
}

/// A component of an *existing* path that we are about to hand to a Windows API: wildcards,
/// stream separators and device names are never passed through.
pub fn check_host_component(name: &str) -> FsResult<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(invalid_path("Invalid path component."));
    }
    if name.chars().any(|c| c.is_control() || c == '/' || c == '\\') {
        return Err(invalid_path("Invalid path component."));
    }
    if let Some(c) = name.chars().find(|c| WINDOWS_FORBIDDEN.contains(c)) {
        return Err(FsError::new(
            FsErrorKind::Unavailable,
            format!("\"{name}\" contains \"{c}\", which Windows file APIs cannot address. Use a terminal for it."),
        ));
    }
    if is_reserved_device(name) {
        return Err(FsError::new(FsErrorKind::Unavailable, format!("\"{name}\" is a reserved Windows device name.")));
    }
    Ok(())
}

/// "a.txt" -> ("a", ".txt"); "archive.tar.gz" -> ("archive.tar", ".gz"); ".bashrc" -> (".bashrc", "").
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_paths_are_normalised() {
        assert_eq!(normalize_linux("/").unwrap(), "/");
        assert_eq!(normalize_linux("//home///me/./docs/").unwrap(), "/home/me/docs");
    }

    #[test]
    fn invalid_linux_paths_are_refused() {
        for p in ["", "relative/path", "~/x", "/a/../b", "/..", "/a\0b", "/a\nb", "/a\\b", "C:\\Windows"] {
            let e = linux_components(p).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::InvalidPath, "{p:?}");
        }
        assert!(linux_components(&format!("/{}", "a".repeat(5000))).is_err());
        assert_eq!(linux_components(&format!("/{}", "a".repeat(300))).unwrap_err().kind, FsErrorKind::InvalidName);
    }

    #[test]
    fn parent_of_a_path() {
        assert_eq!(linux_parent("/home/me").as_deref(), Some("/home"));
        assert_eq!(linux_parent("/home").as_deref(), Some("/"));
        assert_eq!(linux_parent("/"), None);
    }

    #[test]
    fn names_for_new_items() {
        for ok in ["notes.txt", ".bashrc", "my folder", "日本語.txt", "a-b_c (1)"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        let long = "x".repeat(256);
        for bad in ["", ".", "..", "a/b", "a\\b", "a:b", "what?", "star*", "q\"q", "a|b", "<x>", "end.", "end ", " lead", "nul", "COM1.txt", "a\tb", long.as_str()] {
            assert_eq!(check_name(bad).unwrap_err().kind, FsErrorKind::InvalidName, "{bad:?}");
        }
    }

    #[test]
    fn host_components_never_carry_streams_or_wildcards() {
        assert!(check_host_component("file.txt").is_ok());
        assert!(check_host_component("file.txt:stream").is_err());
        assert!(check_host_component("*.txt").is_err());
        assert!(check_host_component("..").is_err());
        assert!(check_host_component("CON").is_err());
    }

    #[test]
    fn extension_splitting() {
        assert_eq!(split_ext("a.txt"), ("a", ".txt"));
        assert_eq!(split_ext("archive.tar.gz"), ("archive.tar", ".gz"));
        assert_eq!(split_ext(".bashrc"), (".bashrc", ""));
        assert_eq!(split_ext("noext"), ("noext", ""));
        assert_eq!(split_ext("trailing."), ("trailing.", ""));
    }
}
