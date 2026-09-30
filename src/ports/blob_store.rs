//! Object storage port. Multipart upload mirrors S3 so large files can be sent in resumable chunks.

use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("object not found")]
    NotFound,
    #[error("invalid range")]
    BadRange,
    #[error("storage error: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInfo {
    pub number: i32,
    pub size: u64,
    pub etag: String,
}

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

pub struct BlobRead {
    /// Size of the whole object.
    pub total: u64,
    /// Inclusive byte range that `stream` yields, `None` for the whole object.
    pub range: Option<(u64, u64)>,
    pub stream: ByteStream,
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn start_multipart(&self, key: &str, content_type: &str) -> Result<String, BlobError>;

    /// Stores one part and returns its etag. Re-sending a part number replaces it.
    async fn put_part(
        &self,
        key: &str,
        upload_id: &str,
        number: i32,
        data: Bytes,
    ) -> Result<String, BlobError>;

    async fn list_parts(&self, key: &str, upload_id: &str) -> Result<Vec<PartInfo>, BlobError>;

    /// Joins the parts (in the order given) and returns the object size.
    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[PartInfo],
    ) -> Result<u64, BlobError>;

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), BlobError>;

    /// Small objects such as thumbnails.
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<(), BlobError>;

    /// `range` is an inclusive `(start, end)`; `end` is clamped to the object size.
    async fn read(&self, key: &str, range: Option<(u64, u64)>) -> Result<BlobRead, BlobError>;

    /// Deleting a missing key is not an error.
    async fn delete_many(&self, keys: &[String]) -> Result<(), BlobError>;
}
