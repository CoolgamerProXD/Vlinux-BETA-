//! Error type returned from Tauri commands. Serialised as a plain string so the
//! frontend can display it directly.

use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Io(String),
    Config(String),
    Window(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Io(m) => write!(f, "I/O error: {m}"),
            AppError::Config(m) => write!(f, "Configuration error: {m}"),
            AppError::Window(m) => write!(f, "Window error: {m}"),
        }
    }
}

impl std::error::Error for AppError {}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Window(e.to_string())
    }
}
