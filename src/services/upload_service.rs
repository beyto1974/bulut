use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use chrono::{DateTime, Duration, Utc};
use futures_util::{Stream, StreamExt};
use uuid::Uuid;

use crate::domain::node::{validate_name, validate_tag, FileVersion, Node};
use crate::domain::upload::{Upload, UploadStatus};
use crate::ports::blob_store::{BlobError, BlobStore, PartInfo};
use crate::ports::clock::Clock;
use crate::ports::node_repo::{NewVersion, NodeRepo};
use crate::ports::upload_repo::UploadRepo;
use crate::services::error::ServiceError;

/// Aborts a multipart upload when dropped while armed. A client that disconnects during a streamed
/// upload makes the server drop the handler at an await point, where no cleanup code would run.
struct AbortOnDrop {
    blobs: Arc<dyn BlobStore>,
    key: String,
    upload_id: String,
    armed: bool,
}

impl AbortOnDrop {
    fn new(blobs: Arc<dyn BlobStore>, key: &str, upload_id: &str) -> Self {
        Self {
            blobs,
            key: key.to_string(),
            upload_id: upload_id.to_string(),
            armed: true,
        }
    }

    /// The normal paths finish or abort the upload themselves.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let (blobs, key, id) = (self.blobs.clone(), self.key.clone(), self.upload_id.clone());
            rt.spawn(async move {
                if let Err(e) = blobs.abort_multipart(&key, &id).await {
                    tracing::warn!(error = %e, "could not abort a dropped upload");
                }
            });
        }
    }
}

pub struct NewUpload {
    pub parent: Option<Uuid>,
    pub name: String,
    pub size: i64,
    pub content_type: Option<String>,
    pub client_created_at: Option<DateTime<Utc>>,
}

pub struct UploadService {
    pub folders_enabled: bool,
    pub uploads: Arc<dyn UploadRepo>,
    pub nodes: Arc<dyn NodeRepo>,
    pub blobs: Arc<dyn BlobStore>,
    pub clock: Arc<dyn Clock>,
    pub part_size: usize,
    pub max_file_bytes: u64,
}

fn clean_content_type(ct: Option<String>) -> String {
    ct.map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty() && c.len() <= 255 && c.is_ascii() && c.contains('/'))
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

impl UploadService {
    fn check_size(&self, size: u64) -> Result<(), ServiceError> {
        if size > self.max_file_bytes {
            return Err(ServiceError::Invalid(format!(
                "file is larger than the limit of {} bytes",
                self.max_file_bytes
            )));
        }
        Ok(())
    }

    async fn check_parent(&self, session: &str, parent: Option<Uuid>) -> Result<(), ServiceError> {
        if parent.is_some() && !self.folders_enabled {
            return Err(ServiceError::Invalid("folders are not enabled".into()));
        }
        if let Some(p) = parent {
            let node = self
                .nodes
                .get(session, p)
                .await?
                .ok_or(ServiceError::NotFound)?;
            if node.kind != crate::domain::node::NodeKind::Folder {
                return Err(ServiceError::Invalid("parent is not a folder".into()));
            }
        }
        Ok(())
    }

    fn status(&self, u: &Upload, received: Vec<i32>) -> UploadStatus {
        UploadStatus {
            upload_id: u.id,
            name: u.name.clone(),
            size: u.size,
            part_size: u.part_size,
            parts_total: u.parts_total(),
            parts_received: received,
        }
    }

    pub async fn init(&self, session: &str, new: NewUpload) -> Result<UploadStatus, ServiceError> {
        let name = validate_name(&new.name)
            .map_err(|m| ServiceError::Invalid(m.into()))?
            .to_string();
        if new.size < 0 {
            return Err(ServiceError::Invalid("size cannot be negative".into()));
        }
        self.check_size(new.size as u64)?;
        self.check_parent(session, new.parent).await?;

        let content_type = clean_content_type(new.content_type);
        let blob_key = format!("{session}/{}", Uuid::new_v4());
        // An empty file has no parts, so it never needs a multipart upload.
        let s3_upload_id = if new.size == 0 {
            String::new()
        } else {
            self.blobs.start_multipart(&blob_key, &content_type).await?
        };
        let upload = Upload {
            id: Uuid::new_v4(),
            session_code: session.to_string(),
            parent_id: new.parent,
            name,
            content_type,
            size: new.size,
            part_size: self.part_size as i32,
            blob_key,
            s3_upload_id,
            client_created_at: new.client_created_at,
            created_at: self.clock.now(),
        };
        if let Err(e) = self.uploads.create(&upload).await {
            let _ = self
                .blobs
                .abort_multipart(&upload.blob_key, &upload.s3_upload_id)
                .await;
            return Err(e.into());
        }
        Ok(self.status(&upload, vec![]))
    }

    async fn load(&self, session: &str, id: Uuid) -> Result<Upload, ServiceError> {
        self.uploads
            .get(session, id)
            .await?
            .ok_or(ServiceError::NotFound)
    }

    /// Which parts are stored, so a client can resume after a dropped connection.
    pub async fn status_of(&self, session: &str, id: Uuid) -> Result<UploadStatus, ServiceError> {
        let u = self.load(session, id).await?;
        let received = self
            .received_parts(&u)
            .await?
            .into_iter()
            .map(|p| p.number)
            .collect();
        Ok(self.status(&u, received))
    }

    async fn received_parts(&self, u: &Upload) -> Result<Vec<PartInfo>, ServiceError> {
        if u.size == 0 {
            return Ok(vec![]);
        }
        match self.blobs.list_parts(&u.blob_key, &u.s3_upload_id).await {
            Ok(mut parts) => {
                parts.sort_by_key(|p| p.number);
                Ok(parts)
            }
            Err(BlobError::NotFound) => Err(ServiceError::Conflict(
                "this upload expired, start a new one".into(),
            )),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn put_part(
        &self,
        session: &str,
        id: Uuid,
        number: i32,
        data: Bytes,
    ) -> Result<(), ServiceError> {
        let u = self.load(session, id).await?;
        let expected = u.expected_part_size(number).ok_or_else(|| {
            ServiceError::Invalid(format!(
                "part {number} is out of range 1..={}",
                u.parts_total()
            ))
        })?;
        if data.len() as i64 != expected {
            return Err(ServiceError::Invalid(format!(
                "part {number} must be {expected} bytes, got {}",
                data.len()
            )));
        }
        self.blobs
            .put_part(&u.blob_key, &u.s3_upload_id, number, data)
            .await?;
        Ok(())
    }

    pub async fn complete(
        &self,
        session: &str,
        id: Uuid,
    ) -> Result<(Node, FileVersion), ServiceError> {
        let u = self.load(session, id).await?;
        if u.size == 0 {
            self.blobs
                .put(&u.blob_key, Bytes::new(), &u.content_type)
                .await?;
        } else {
            let parts = self.received_parts(&u).await?;
            let total = u.parts_total();
            let missing: Vec<i32> = (1..=total)
                .filter(|n| !parts.iter().any(|p| p.number == *n))
                .collect();
            if !missing.is_empty() {
                return Err(ServiceError::Conflict(format!(
                    "{} part(s) still missing, first is {}",
                    missing.len(),
                    missing[0]
                )));
            }
            self.blobs
                .complete_multipart(&u.blob_key, &u.s3_upload_id, &parts)
                .await?;
        }
        let added = self
            .nodes
            .add_version(
                session,
                u.parent_id,
                &u.name,
                NewVersion {
                    size: u.size,
                    content_type: u.content_type.clone(),
                    sha256: None,
                    blob_key: u.blob_key.clone(),
                    thumb_key: None,
                    client_created_at: u.client_created_at,
                },
                self.clock.now(),
            )
            .await;
        match added {
            Ok(pair) => {
                self.uploads.delete(u.id).await?;
                Ok(pair)
            }
            Err(e) => {
                // The stored object has no row pointing to it, so remove it.
                let _ = self
                    .blobs
                    .delete_many(std::slice::from_ref(&u.blob_key))
                    .await;
                let _ = self.uploads.delete(u.id).await;
                Err(e.into())
            }
        }
    }

    pub async fn abort(&self, session: &str, id: Uuid) -> Result<(), ServiceError> {
        let u = self.load(session, id).await?;
        self.discard(&u).await;
        Ok(())
    }

    async fn discard(&self, u: &Upload) {
        if u.size > 0 {
            if let Err(e) = self
                .blobs
                .abort_multipart(&u.blob_key, &u.s3_upload_id)
                .await
            {
                tracing::warn!(upload = %u.id, error = %e, "could not abort multipart upload");
            }
        }
        let _ = self.uploads.delete(u.id).await;
    }

    /// Aborts every unfinished upload of a session, used when the session is purged.
    pub async fn discard_session(&self, session: &str) -> Result<(), ServiceError> {
        for u in self.uploads.for_session(session).await? {
            self.discard(&u).await;
        }
        Ok(())
    }

    /// Aborts uploads that were started more than `older_than` ago and never completed.
    pub async fn cleanup_stale(&self, older_than: Duration) -> Result<usize, ServiceError> {
        let stale = self
            .uploads
            .started_before(self.clock.now() - older_than)
            .await?;
        for u in &stale {
            self.discard(u).await;
        }
        Ok(stale.len())
    }

    /// One-request upload for clients that cannot chunk (curl, agents). The body is cut into parts
    /// as it arrives, so the file is never held in memory as a whole.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_stream<S, E>(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        content_type: Option<String>,
        client_created_at: Option<DateTime<Utc>>,
        tags: &[String],
        mut body: S,
    ) -> Result<(Node, FileVersion), ServiceError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
        E: std::fmt::Display,
    {
        let name = validate_name(name)
            .map_err(|m| ServiceError::Invalid(m.into()))?
            .to_string();
        let tags: Vec<String> = tags
            .iter()
            .map(|t| validate_tag(t).map_err(|m| ServiceError::Invalid(m.into())))
            .collect::<Result<_, _>>()?;
        self.check_parent(session, parent).await?;
        let content_type = clean_content_type(content_type);
        let key = format!("{session}/{}", Uuid::new_v4());
        let upload_id = self.blobs.start_multipart(&key, &content_type).await?;
        let mut guard = AbortOnDrop::new(self.blobs.clone(), &key, &upload_id);

        let result = async {
            let mut parts: Vec<PartInfo> = Vec::new();
            let mut total: u64 = 0;
            let mut buf = BytesMut::with_capacity(self.part_size);
            loop {
                let next = body.next().await;
                let done = next.is_none();
                if let Some(chunk) = next {
                    let chunk = chunk
                        .map_err(|e| ServiceError::Invalid(format!("upload interrupted: {e}")))?;
                    total += chunk.len() as u64;
                    self.check_size(total)?;
                    buf.extend_from_slice(&chunk);
                }
                while buf.len() >= self.part_size || (done && !buf.is_empty()) {
                    let take = buf.len().min(self.part_size);
                    let data = buf.split_to(take).freeze();
                    let number = parts.len() as i32 + 1;
                    let size = data.len() as u64;
                    let etag = self.blobs.put_part(&key, &upload_id, number, data).await?;
                    parts.push(PartInfo { number, size, etag });
                }
                if done {
                    break;
                }
            }
            if parts.is_empty() {
                // Nothing was sent: store an empty object instead of a multipart upload.
                let _ = self.blobs.abort_multipart(&key, &upload_id).await;
                self.blobs.put(&key, Bytes::new(), &content_type).await?;
            } else {
                self.blobs
                    .complete_multipart(&key, &upload_id, &parts)
                    .await?;
            }
            Ok::<u64, ServiceError>(total)
        }
        .await;
        // From here the code below finishes or aborts the upload itself.
        guard.disarm();

        let total = match result {
            Ok(t) => t,
            Err(e) => {
                let _ = self.blobs.abort_multipart(&key, &upload_id).await;
                return Err(e);
            }
        };
        let added = self
            .nodes
            .add_version(
                session,
                parent,
                &name,
                NewVersion {
                    size: total as i64,
                    content_type,
                    sha256: None,
                    blob_key: key.clone(),
                    thumb_key: None,
                    client_created_at,
                },
                self.clock.now(),
            )
            .await;
        let (node, mut version) = match added {
            Ok(pair) => pair,
            Err(e) => {
                let _ = self.blobs.delete_many(&[key]).await;
                return Err(e.into());
            }
        };
        for tag in &tags {
            version = self.nodes.add_tag(session, version.id, tag).await?;
        }
        Ok((node, version))
    }
}
