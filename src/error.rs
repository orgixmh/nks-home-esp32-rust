use std::fmt::{Display, Formatter};
use esp_idf_svc::sys::EspError;

#[derive(Debug)]
pub enum AppError {
    Message(String),
    Esp(EspError),
    Utf8(std::string::FromUtf8Error),
    ParseInt(std::num::ParseIntError),
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => write!(f, "{msg}"),
            Self::Esp(e) => write!(f, "ESP error: {e}"),
            Self::Utf8(e) => write!(f, "UTF-8 error: {e}"),
            Self::ParseInt(e) => write!(f, "Parse int error: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<EspError> for AppError {
    fn from(value: EspError) -> Self {
        Self::Esp(value)
    }
}

impl From<std::string::FromUtf8Error> for AppError {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<std::num::ParseIntError> for AppError {
    fn from(value: std::num::ParseIntError) -> Self {
        Self::ParseInt(value)
    }
}
