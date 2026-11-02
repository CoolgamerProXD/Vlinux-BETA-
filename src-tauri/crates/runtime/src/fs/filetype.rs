//! File classification (icon / label in the UI) and the policy for "Open".
//!
//! "Open" hands a file to the user's *Windows* default application (a Linux file is reached through
//! its `\\wsl.localhost\...` path). Types that Windows would *run* rather than *open* (programs,
//! scripts, installers, shortcuts, registry files...) are never launched from here; the file
//! manager offers "Show in Explorer" for them instead.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FileClass {
    pub id: &'static str,
    pub label: &'static str,
}

const fn c(id: &'static str, label: &'static str) -> FileClass {
    FileClass { id, label }
}

pub const FOLDER: FileClass = c("folder", "Folder");
pub const LINK: FileClass = c("link", "Link");
pub const OTHER: FileClass = c("other", "Special file");
pub const GENERIC: FileClass = c("file", "File");

pub fn extension(name: &str) -> Option<String> {
    let trimmed = name.trim_end_matches([' ', '.']);
    let (_, ext) = super::path::split_ext(trimmed);
    if ext.is_empty() {
        None
    } else {
        Some(ext[1..].to_ascii_lowercase())
    }
}

pub fn classify_file(name: &str) -> FileClass {
    let Some(ext) = extension(name) else {
        return match name {
            "Makefile" | "Dockerfile" | "LICENSE" | "README" | "PKGBUILD" => c("code", "Source or config"),
            _ => GENERIC,
        };
    };
    match ext.as_str() {
        "txt" | "log" | "text" | "rtf" => c("text", "Text document"),
        "md" | "markdown" | "rst" => c("text", "Markup document"),
        "pdf" => c("pdf", "PDF document"),
        "doc" | "docx" | "odt" | "pages" => c("document", "Word-processing document"),
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => c("spreadsheet", "Spreadsheet"),
        "ppt" | "pptx" | "odp" | "key" => c("presentation", "Presentation"),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff" | "heic" | "avif" => c("image", "Image"),
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" => c("audio", "Audio"),
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "m4v" | "flv" => c("video", "Video"),
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" => c("archive", "Archive"),
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "qcow2" => c("disk", "Disk image"),
        "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "go" | "java" | "kt" | "swift" | "cs" | "py" | "rb" | "php" | "lua" | "pl"
        | "js" | "mjs" | "ts" | "tsx" | "jsx" | "svelte" | "vue" | "html" | "htm" | "css" | "scss" | "sql" | "sh" | "bash" | "zsh"
        | "fish" | "ps1" | "bat" | "cmd" | "toml" | "yaml" | "yml" | "json" | "xml" | "ini" | "conf" | "cfg" | "lock" | "diff" | "patch" => {
            c("code", "Source or config")
        }
        "exe" | "msi" | "com" | "scr" | "dll" | "sys" | "appx" | "msix" => c("program", "Windows program"),
        "ttf" | "otf" | "woff" | "woff2" => c("font", "Font"),
        "db" | "sqlite" | "sqlite3" | "mdb" => c("database", "Database"),
        "lnk" | "url" => c("shortcut", "Windows shortcut"),
        _ => GENERIC,
    }
}

/// Extensions Windows would execute, install or merge into the system when opened.
const BLOCKED: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "ps1", "psm1", "ps1xml", "psc1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta",
    "msi", "msp", "mst", "lnk", "url", "reg", "inf", "scf", "cpl", "dll", "sys", "jar", "appx", "msix", "appinstaller",
    "application", "gadget", "msc", "py", "pyw", "pyz", "ocx", "drv", "vb", "wsc", "theme",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenPolicy {
    pub allowed: bool,
    pub reason: Option<String>,
}

pub fn open_policy(name: &str) -> OpenPolicy {
    match extension(name) {
        Some(ext) if BLOCKED.contains(&ext.as_str()) => OpenPolicy {
            allowed: false,
            reason: Some(format!(
                "\".{ext}\" files can run code or change Windows, so VLINUX does not launch them from the file manager. Use \"Show in Explorer\" if you really want to."
            )),
        },
        _ => OpenPolicy { allowed: true, reason: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_types_are_classified() {
        assert_eq!(classify_file("photo.JPG").id, "image");
        assert_eq!(classify_file("report.docx").id, "document");
        assert_eq!(classify_file("data.csv").id, "spreadsheet");
        assert_eq!(classify_file("backup.tar.gz").id, "archive");
        assert_eq!(classify_file("main.rs").id, "code");
        assert_eq!(classify_file("song.flac").id, "audio");
        assert_eq!(classify_file("Makefile").id, "code");
        assert_eq!(classify_file("noextension").id, "file");
        assert_eq!(classify_file(".bashrc").id, "file");
    }

    #[test]
    fn programs_and_scripts_are_never_launched() {
        for n in ["setup.exe", "run.BAT", "x.cmd", "a.ps1", "b.vbs", "c.js", "d.msi", "e.lnk", "f.reg", "g.py", "evil.pdf.exe", "trailing.exe. ", "h.jar"] {
            assert!(!open_policy(n).allowed, "{n}");
            assert!(open_policy(n).reason.is_some());
        }
        for n in ["a.txt", "b.pdf", "c.png", "d.sh", "e.docx", "noext", "f.md"] {
            assert!(open_policy(n).allowed, "{n}");
        }
    }
}
