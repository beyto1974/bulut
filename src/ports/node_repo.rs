use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::node::{FileVersion, Node, NodeEntry};
use crate::ports::session_repo::RepoError;

/// What the upload service hands over once all bytes are stored.
#[derive(Debug, Clone)]
pub struct NewVersion {
    pub size: i64,
    pub content_type: String,
    pub sha256: Option<String>,
    pub blob_key: String,
    pub thumb_key: Option<String>,
    pub client_created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct NodePatch {
    pub name: Option<String>,
    pub note: Option<String>,
}

/// Storage of the file tree: folders, files, versions and tags.
#[async_trait]
pub trait NodeRepo: Send + Sync {
    /// `RepoError::NameTaken` if the name exists in that folder.
    async fn create_folder(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<Node, RepoError>;

    /// Adds a version to the file called `name` in `parent`, creating the file if needed.
    /// The previous latest version stays, only the `is_latest` flag moves.
    async fn add_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        new: NewVersion,
        now: DateTime<Utc>,
    ) -> Result<(Node, FileVersion), RepoError>;

    async fn get(&self, session: &str, id: Uuid) -> Result<Option<Node>, RepoError>;

    /// Children of `parent` (or of the session root), folders first, then by name.
    async fn list(&self, session: &str, parent: Option<Uuid>) -> Result<Vec<NodeEntry>, RepoError>;

    async fn update(&self, session: &str, id: Uuid, patch: NodePatch) -> Result<Node, RepoError>;

    /// Deletes a node and everything under it. Returns the storage keys (blobs and thumbnails)
    /// that the caller must remove from object storage.
    async fn delete(&self, session: &str, id: Uuid) -> Result<Vec<String>, RepoError>;

    /// Every storage key of a session, used when the whole session is purged.
    async fn all_keys(&self, session: &str) -> Result<Vec<String>, RepoError>;

    /// Newest first.
    async fn versions(&self, session: &str, node_id: Uuid) -> Result<Vec<FileVersion>, RepoError>;

    async fn version(
        &self,
        session: &str,
        version_id: Uuid,
    ) -> Result<Option<(Node, FileVersion)>, RepoError>;

    /// Puts `tag` on a version. If another version of the same file has it, the tag moves.
    async fn add_tag(
        &self,
        session: &str,
        version_id: Uuid,
        tag: &str,
    ) -> Result<FileVersion, RepoError>;

    async fn remove_tag(&self, session: &str, version_id: Uuid, tag: &str)
        -> Result<(), RepoError>;

    /// Finds the file version for a path filter, used by `?name=&tag=`.
    /// `tag = None` returns the latest version.
    async fn find_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        tag: Option<&str>,
    ) -> Result<Option<(Node, FileVersion)>, RepoError>;

    async fn set_thumb_key(&self, version_id: Uuid, key: &str) -> Result<(), RepoError>;
}
