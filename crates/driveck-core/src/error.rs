use std::{fmt, io};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Error)]
#[error("{message}")]
pub struct DriveCkError {
    pub message: String,
}

impl DriveCkError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn io(message: impl fmt::Display, error: io::Error) -> Self {
        Self::new(format!("{message}: {error}"))
    }
}

impl From<io::Error> for DriveCkError {
    fn from(value: io::Error) -> Self {
        Self::new(value.to_string())
    }
}
