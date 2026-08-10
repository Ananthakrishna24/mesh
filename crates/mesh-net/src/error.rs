use std::fmt::{Display, Formatter, Result as FmtResult};

pub type NetResult<T> = Result<T, NetError>;

#[derive(Debug)]
pub enum NetError {
    Io(std::io::Error),
    Quic(quinn::ConnectError),
    Connection(quinn::ConnectionError),
    Write(quinn::WriteError),
    Read(quinn::ReadExactError),
    Tls(String),
    Protocol(String),
    Identity(String),
    Closed,
    Timeout,
    Core(mesh_core::CoreError),
}

impl Display for NetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Io(error) => write!(f, "network io error: {error}"),
            Self::Quic(error) => write!(f, "quic connect error: {error}"),
            Self::Connection(error) => write!(f, "quic connection error: {error}"),
            Self::Write(error) => write!(f, "quic write error: {error}"),
            Self::Read(error) => write!(f, "quic read error: {error}"),
            Self::Tls(message) => write!(f, "tls error: {message}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Identity(message) => write!(f, "identity error: {message}"),
            Self::Closed => write!(f, "connection closed"),
            Self::Timeout => write!(f, "network operation timed out"),
            Self::Core(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<std::io::Error> for NetError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<quinn::ConnectError> for NetError {
    fn from(value: quinn::ConnectError) -> Self {
        Self::Quic(value)
    }
}

impl From<quinn::ConnectionError> for NetError {
    fn from(value: quinn::ConnectionError) -> Self {
        Self::Connection(value)
    }
}

impl From<quinn::WriteError> for NetError {
    fn from(value: quinn::WriteError) -> Self {
        Self::Write(value)
    }
}

impl From<quinn::ReadExactError> for NetError {
    fn from(value: quinn::ReadExactError) -> Self {
        Self::Read(value)
    }
}

impl From<mesh_core::CoreError> for NetError {
    fn from(value: mesh_core::CoreError) -> Self {
        Self::Core(value)
    }
}
