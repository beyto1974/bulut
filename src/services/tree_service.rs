use std::sync::Arc;

use uuid::Uuid;

use crate::domain::node::{validate_name, validate_tag, FileVersion, Node, NodeEntry, NodeKind};
use crate::ports::blob_store::{BlobRead, BlobStore};
use crate::ports::clock::Clock;
use crate::ports::node_repo::{NodePatch, NodeRepo};
use crate::services::error::ServiceError;

const MAX_NOTE: usize = 4000;

/// One row of [`TreeService::walk`].
pub struct WalkItem {
    /// Path from the session root.
    pub path: String,
    pub entry: NodeEntry,
    /// Newest first. Empty for folders.
    pub versions: Vec<FileVersion>,
}

pub struct Walk {
    pub items: Vec<WalkItem>,
    pub truncated: bool,
}

pub struct TreeService {
    pub folders_enabled: bool,
    /// Most tags per version.
    pub max_tags: u32,
    pub nodes: Arc<dyn NodeRepo>,
    pub blobs: Arc<dyn BlobStore>,
    pub clock: Arc<dyn Clock>,
}

impl TreeService {
    pub async fn list(
        &self,
        session: &str,
        parent: Option<Uuid>,
    ) -> Result<Vec<NodeEntry>, ServiceError> {
        if let Some(p) = parent {
            self.nodes
                .get(session, p)
                .await?
                .ok_or(ServiceError::NotFound)?;
        }
        Ok(self.nodes.list(session, parent).await?)
    }

    /// Every node of a session in display order (folders first, contents right after their
    /// folder), with file versions, for the text index. Stops at `limit` nodes and says so.
    pub async fn walk(&self, session: &str, limit: usize) -> Result<Walk, ServiceError> {
        let mut walk = Walk {
            items: Vec::new(),
            truncated: false,
        };
        self.walk_dir(session, None, String::new(), limit, &mut walk)
            .await?;
        Ok(walk)
    }

    fn walk_dir<'a>(
        &'a self,
        session: &'a str,
        parent: Option<Uuid>,
        prefix: String,
        limit: usize,
        walk: &'a mut Walk,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ServiceError>> + Send + 'a>>
    {
        Box::pin(async move {
            let entries = self.nodes.list(session, parent).await?;
            // One query for the versions of every file that still fits under the limit, instead
            // of one per file.
            let room = limit.saturating_sub(walk.items.len());
            let file_ids: Vec<Uuid> = entries
                .iter()
                .take(room)
                .filter(|e| e.node.kind != NodeKind::Folder)
                .map(|e| e.node.id)
                .collect();
            let mut versions_by_file = self.nodes.versions_of(session, &file_ids).await?;
            for entry in entries {
                if walk.items.len() >= limit {
                    walk.truncated = true;
                    return Ok(());
                }
                let path = format!("{prefix}{}", entry.node.name);
                let is_folder = entry.node.kind == NodeKind::Folder;
                let versions = if is_folder {
                    Vec::new()
                } else {
                    versions_by_file.remove(&entry.node.id).unwrap_or_default()
                };
                let id = entry.node.id;
                walk.items.push(WalkItem {
                    path: path.clone(),
                    entry,
                    versions,
                });
                if is_folder {
                    self.walk_dir(session, Some(id), format!("{path}/"), limit, walk)
                        .await?;
                    if walk.truncated {
                        return Ok(());
                    }
                }
            }
            Ok(())
        })
    }

    /// Path from the root to `id`, for breadcrumbs. Empty for the root.
    pub async fn path(&self, session: &str, id: Option<Uuid>) -> Result<Vec<Node>, ServiceError> {
        let mut out = Vec::new();
        let mut cur = id;
        while let Some(id) = cur {
            let node = self
                .nodes
                .get(session, id)
                .await?
                .ok_or(ServiceError::NotFound)?;
            cur = node.parent_id;
            out.push(node);
            if out.len() > 64 {
                return Err(ServiceError::Invalid("folder nesting is too deep".into()));
            }
        }
        out.reverse();
        Ok(out)
    }

    pub async fn create_folder(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
    ) -> Result<Node, ServiceError> {
        if !self.folders_enabled {
            return Err(ServiceError::Invalid("folders are not enabled".into()));
        }
        let name = validate_name(name).map_err(|m| ServiceError::Invalid(m.into()))?;
        Ok(self
            .nodes
            .create_folder(session, parent, name, self.clock.now())
            .await?)
    }

    pub async fn update(
        &self,
        session: &str,
        id: Uuid,
        name: Option<&str>,
        note: Option<&str>,
    ) -> Result<Node, ServiceError> {
        let name = match name {
            Some(n) => Some(
                validate_name(n)
                    .map_err(|m| ServiceError::Invalid(m.into()))?
                    .to_string(),
            ),
            None => None,
        };
        let note = note.map(|n| n.trim().to_string());
        if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE) {
            return Err(ServiceError::Invalid(format!(
                "note is longer than {MAX_NOTE} characters"
            )));
        }
        Ok(self
            .nodes
            .update(session, id, NodePatch { name, note })
            .await?)
    }

    pub async fn delete(&self, session: &str, id: Uuid) -> Result<(), ServiceError> {
        let keys = self.nodes.delete(session, id).await?;
        // Rows are gone; a failed object delete leaves orphans, which is logged, not surfaced.
        if let Err(e) = self.blobs.delete_many(&keys).await {
            tracing::error!(error = %e, keys = keys.len(), "failed to delete stored objects");
        }
        Ok(())
    }

    pub async fn versions(
        &self,
        session: &str,
        node: Uuid,
    ) -> Result<Vec<FileVersion>, ServiceError> {
        self.nodes
            .get(session, node)
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(self.nodes.versions(session, node).await?)
    }

    /// Looks up one version by id, without opening its bytes.
    pub async fn resolve_version(
        &self,
        session: &str,
        version: Uuid,
    ) -> Result<(Node, FileVersion), ServiceError> {
        self.nodes
            .version(session, version)
            .await?
            .ok_or(ServiceError::NotFound)
    }

    /// Finds a file by name (and optionally tag). Without a tag, or with `latest`, this is the
    /// newest version.
    pub async fn resolve_named(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        tag: Option<&str>,
    ) -> Result<(Node, FileVersion), ServiceError> {
        self.nodes
            .find_version(session, parent, name, tag)
            .await?
            .ok_or(ServiceError::NotFound)
    }

    /// Opens the bytes of a version. `range` is an inclusive `(start, end)`.
    pub async fn open(
        &self,
        version: &FileVersion,
        range: Option<(u64, u64)>,
    ) -> Result<BlobRead, ServiceError> {
        Ok(self.blobs.read(&version.blob_key, range).await?)
    }

    pub async fn add_tag(
        &self,
        session: &str,
        version: Uuid,
        tag: &str,
    ) -> Result<FileVersion, ServiceError> {
        let tag = validate_tag(tag).map_err(|m| ServiceError::Invalid(m.into()))?;
        // Tags stored on the version. `latest` is added on top for the newest one and is not stored.
        let (_, current) = self.resolve_version(session, version).await?;
        let stored = current
            .tags
            .iter()
            .filter(|t| t.as_str() != "latest")
            .count();
        if !current.tags.contains(&tag) && stored >= self.max_tags as usize {
            return Err(ServiceError::Conflict(format!(
                "a version can have at most {} tags, remove one first",
                self.max_tags
            )));
        }
        Ok(self.nodes.add_tag(session, version, &tag).await?)
    }

    pub async fn remove_tag(
        &self,
        session: &str,
        version: Uuid,
        tag: &str,
    ) -> Result<(), ServiceError> {
        Ok(self.nodes.remove_tag(session, version, tag).await?)
    }
}
