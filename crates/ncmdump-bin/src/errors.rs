use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum Error {
    #[error("Can't resolve the path")]
    Path,
    #[error("No file can be converted")]
    NoFile,
    #[error("Worker can't less than 0 and more than 8")]
    Worker,
}

#[derive(Clone, Debug, Error)]
pub enum DumpError {
    #[error("Invalid file format")]
    Format,
    #[error("Output file already exists")]
    Exists,
}
