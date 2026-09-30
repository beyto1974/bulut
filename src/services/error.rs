use crate::ports::blob_store::BlobError;
use crate::ports::session_repo::RepoError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("storage error")]
    Storage(String),
}

impl From<RepoError> for ServiceError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::NotFound => Self::NotFound,
            RepoError::NameTaken => {
                Self::Conflict("a file or folder with this name already exists here".into())
            }
            RepoError::LimitReached(message) => Self::Conflict(message),
            RepoError::CodeTaken => Self::Conflict("code already taken".into()),
            RepoError::Invalid(m) => Self::Invalid(m),
            RepoError::Storage(m) => Self::Storage(m),
        }
    }
}

impl From<BlobError> for ServiceError {
    fn from(e: BlobError) -> Self {
        match e {
            BlobError::NotFound => Self::NotFound,
            BlobError::BadRange => Self::Invalid("invalid range".into()),
            BlobError::Storage(m) => Self::Storage(m),
        }
    }
}
