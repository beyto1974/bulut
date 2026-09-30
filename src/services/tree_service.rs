use std::sync::Arc;

use uuid::Uuid;

use crate::domain::node::{validate_name, validate_tag, FileVersion, Node, NodeEntry};
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::Clock;
use crate::ports::node_repo::{NodePatch, NodeRepo};
use crate::services::error::ServiceError;

const MAX_NOTE: usize = 4000;

pub struct TreeService {
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

    pub async fn add_tag(
        &self,
        session: &str,
        version: Uuid,
        tag: &str,
    ) -> Result<FileVersion, ServiceError> {
        let tag = validate_tag(tag).map_err(|m| ServiceError::Invalid(m.into()))?;
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
