use std::io;

use ncmdump::error::Errors;
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Error {
    #[error("Unknow error")]
    Unknow,
    #[error("Can't resolve the path: {0}")]
    Path(String),
    #[error("Invalid file format")]
    Format,
    #[error("No target can be converted")]
    NoTarget,
    #[error("Can't get file's metadata")]
    Metadata,
    #[error("Worker can't less than 0 and more than 8")]
    Worker,
    #[error("Dump err: {0}")]
    Dump(String),
    #[error("Output file already exists")]
    Exists,
}

impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::Unknow
    }
}

impl From<Errors> for Error {
    fn from(err: Errors) -> Self {
        Error::Dump(err.to_string())
    }
}
