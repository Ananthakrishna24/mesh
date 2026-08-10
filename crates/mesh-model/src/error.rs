use std::fmt::{Display, Formatter, Result as FmtResult};

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Debug)]
pub enum ModelError {
    Invalid(String),
    Unsupported(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Core(mesh_core::CoreError),
}

impl Display for ModelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Invalid(message) => write!(f, "invalid model data: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported model feature: {message}"),
            Self::Io(error) => write!(f, "model io error: {error}"),
            Self::Json(error) => write!(f, "model json error: {error}"),
            Self::Core(error) => write!(f, "{error}"),
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
