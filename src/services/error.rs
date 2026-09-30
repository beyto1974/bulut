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
            RepoError::LimitReached(n) => Self::Conflict(format!(
                "this session already holds the maximum of {n} files, delete one or start another session"
            )),
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
