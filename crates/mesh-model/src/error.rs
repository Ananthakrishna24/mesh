use std::fmt::{Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Debug)]
pub enum ModelError {
    Invalid(String),
    Unsupported(String),
    Access(String),
    NotFound(String),
    Provider(String),
    Http(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Core(mesh_core::CoreError),
    Cancelled,
    Path(PathBuf, String),
}

impl Display for ModelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Invalid(message) => write!(f, "invalid model data: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported model feature: {message}"),
            Self::Access(message) => write!(f, "provider access error: {message}"),
            Self::NotFound(message) => write!(f, "model not found: {message}"),
            Self::Provider(message) => write!(f, "provider error: {message}"),
            Self::Http(message) => write!(f, "model http error: {message}"),
            Self::Io(error) => write!(f, "model io error: {error}"),
            Self::Json(error) => write!(f, "model json error: {error}"),
            Self::Core(error) => write!(f, "{error}"),
            Self::Cancelled => write!(f, "model work cancelled"),
            Self::Path(path, message) => {
                write!(f, "model path error at {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ModelError {}

impl From<std::io::Error> for ModelError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ModelError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<mesh_core::CoreError> for ModelError {
    fn from(value: mesh_core::CoreError) -> Self {
        Self::Core(value)
    }
}

impl From<reqwest::Error> for ModelError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value.to_string())
    }
}
