use std::fmt::{Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Path(String),
    Corrupt(String),
    NewerSchema { found: i32, supported: i32 },
    NotFound(String),
    Core(mesh_core::CoreError),
    Backup(PathBuf, String),
    CredentialStore(String),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Io(error) => write!(f, "store io error: {error}"),
            Self::Sqlite(error) => write!(f, "sqlite error: {error}"),
            Self::Path(message) => write!(f, "store path error: {message}"),
            Self::Corrupt(message) => write!(f, "corrupt store: {message}"),
            Self::NewerSchema { found, supported } => write!(
                f,
                "database schema {found} is newer than supported {supported}"
            ),
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::Core(error) => write!(f, "{error}"),
            Self::Backup(path, message) => {
                write!(f, "migration failed ({message}); backup at {}", path.display())
            }
            Self::CredentialStore(message) => {
                write!(f, "credential store unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

impl From<mesh_core::CoreError> for StoreError {
    fn from(value: mesh_core::CoreError) -> Self {
        Self::Core(value)
    }
}
