//! A database failure right after the object store assembled the file must not cost the client the
//! upload: the row stays, and `complete` can be called again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::adapters::postgres_nodes::PgNodeRepo;
use crate::domain::node::{FileVersion, Node, NodeEntry};
use crate::ports::blob_store::BlobStore;
use crate::ports::node_repo::{NewVersion, NodePatch, NodeRepo};
use crate::ports::session_repo::RepoError;
use crate::services::error::ServiceError;
use crate::services::tests::{fixture, Fixture};
use crate::services::upload_service::{NewUpload, UploadService};

/// The real repository, except that `add_version` fails once with the error it is given.
struct FlakyNodes {
    inner: Arc<PgNodeRepo>,
    fail_next_add: Mutex<Option<RepoError>>,
}

#[async_trait]
impl NodeRepo for FlakyNodes {
    async fn create_folder(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<Node, RepoError> {
        self.inner.create_folder(session, parent, name, now).await
    }

    async fn add_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        new: NewVersion,
        now: DateTime<Utc>,
    ) -> Result<(Node, FileVersion), RepoError> {
        if let Some(e) = self.fail_next_add.lock().unwrap().take() {
            return Err(e);
        }
        self.inner
            .add_version(session, parent, name, new, now)
            .await
    }

    async fn get(&self, session: &str, id: Uuid) -> Result<Option<Node>, RepoError> {
        self.inner.get(session, id).await
    }

    async fn list(&self, session: &str, parent: Option<Uuid>) -> Result<Vec<NodeEntry>, RepoError> {
        self.inner.list(session, parent).await
    }

    async fn update(&self, session: &str, id: Uuid, patch: NodePatch) -> Result<Node, RepoError> {
        self.inner.update(session, id, patch).await
    }

    async fn delete(&self, session: &str, id: Uuid) -> Result<Vec<String>, RepoError> {
        self.inner.delete(session, id).await
    }

    async fn all_keys(&self, session: &str) -> Result<Vec<String>, RepoError> {
        self.inner.all_keys(session).await
    }

    async fn versions(&self, session: &str, node_id: Uuid) -> Result<Vec<FileVersion>, RepoError> {
        self.inner.versions(session, node_id).await
    }

    async fn versions_of(
        &self,
        session: &str,
        node_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<FileVersion>>, RepoError> {
        self.inner.versions_of(session, node_ids).await
    }

    async fn version(
        &self,
        session: &str,
        version_id: Uuid,
    ) -> Result<Option<(Node, FileVersion)>, RepoError> {
        self.inner.version(session, version_id).await
    }

    async fn add_tag(
        &self,
        session: &str,
        version_id: Uuid,
        tag: &str,
    ) -> Result<FileVersion, RepoError> {
        self.inner.add_tag(session, version_id, tag).await
    }

    async fn remove_tag(
        &self,
        session: &str,
        version_id: Uuid,
        tag: &str,
    ) -> Result<(), RepoError> {
        self.inner.remove_tag(session, version_id, tag).await
    }

    async fn find_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        tag: Option<&str>,
    ) -> Result<Option<(Node, FileVersion)>, RepoError> {
        self.inner.find_version(session, parent, name, tag).await
    }

    async fn count_files(&self, session: &str) -> Result<i64, RepoError> {
        self.inner.count_files(session).await
    }

    async fn set_thumb_key(&self, version_id: Uuid, key: &str) -> Result<(), RepoError> {
        self.inner.set_thumb_key(version_id, key).await
    }
}

fn flaky_uploads(f: &Fixture) -> (UploadService, Arc<FlakyNodes>) {
    let flaky = Arc::new(FlakyNodes {
        inner: f.nodes.clone(),
        fail_next_add: Mutex::new(None),
    });
    let svc = UploadService {
        folders_enabled: true,
        uploads: f.uploads.uploads.clone(),
        nodes: flaky.clone(),
        blobs: f.blobs.clone(),
        clock: f.clock.clone(),
        part_size: 10,
        max_file_bytes: 1000,
        max_files: 1000,
        max_pending: 1000,
        max_tags: 1000,
    };
    (svc, flaky)
}

fn new_upload(name: &str, size: i64) -> NewUpload {
    NewUpload {
        parent: None,
        name: name.into(),
        size,
        content_type: None,
        client_created_at: None,
    }
}

async fn read_all(f: &Fixture, key: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut s = f.blobs.read(key, None).await.unwrap().stream;
    while let Some(c) = s.next().await {
        out.extend_from_slice(&c.unwrap());
    }
    out
}

#[tokio::test]
async fn a_database_failure_after_assembly_keeps_the_upload_and_complete_can_be_retried() {
    let f = fixture().await;
    let (svc, flaky) = flaky_uploads(&f);
    let s = f.sessions.create("").await.unwrap();
    let st = svc.init(&s.code, new_upload("big.bin", 25)).await.unwrap();
    for (n, data) in [
        (1, &b"0123456789"[..]),
        (2, &b"abcdefghij"[..]),
        (3, &b"KLMNO"[..]),
    ] {
        svc.put_part(&s.code, st.upload_id, n, Bytes::copy_from_slice(data))
            .await
            .unwrap();
    }

    *flaky.fail_next_add.lock().unwrap() = Some(RepoError::Storage("connection reset".into()));
    let err = svc.complete(&s.code, st.upload_id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Storage(_)), "{err}");

    // Nothing was thrown away: the client still sees every part as received.
    let status = svc.status_of(&s.code, st.upload_id).await.unwrap();
    assert_eq!(status.parts_received, [1, 2, 3]);
    assert!(f.nodes.count_files(&s.code).await.unwrap() == 0);

    let (node, version) = svc.complete(&s.code, st.upload_id).await.unwrap();
    assert_eq!(node.name, "big.bin");
    assert_eq!(version.size, 25);
    assert_eq!(
        read_all(&f, &version.blob_key).await,
        b"0123456789abcdefghijKLMNO"
    );
    // The upload is finished now, so a third call finds nothing.
    assert!(matches!(
        svc.complete(&s.code, st.upload_id).await,
        Err(ServiceError::NotFound)
    ));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn an_empty_file_can_be_retried_too() {
    let f = fixture().await;
    let (svc, flaky) = flaky_uploads(&f);
    let s = f.sessions.create("").await.unwrap();
    let st = svc.init(&s.code, new_upload("empty", 0)).await.unwrap();

    *flaky.fail_next_add.lock().unwrap() = Some(RepoError::Storage("connection reset".into()));
    assert!(svc.complete(&s.code, st.upload_id).await.is_err());
    let (_, version) = svc.complete(&s.code, st.upload_id).await.unwrap();
    assert_eq!(version.size, 0);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_refusal_that_can_never_succeed_still_discards_the_upload_and_the_object() {
    let f = fixture().await;
    let (svc, flaky) = flaky_uploads(&f);
    let s = f.sessions.create("").await.unwrap();
    let st = svc.init(&s.code, new_upload("a.txt", 3)).await.unwrap();
    svc.put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await
        .unwrap();

    *flaky.fail_next_add.lock().unwrap() = Some(RepoError::files_limit(1));
    let err = svc.complete(&s.code, st.upload_id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Conflict(_)), "{err}");
    // Retrying cannot help, so nothing is kept: no row and no object.
    assert!(f
        .uploads
        .uploads
        .for_session(&s.code)
        .await
        .unwrap()
        .is_empty());
    assert!(f.blobs.keys().is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn an_abandoned_retry_is_cleaned_up_with_its_object() {
    let f = fixture().await;
    let (svc, flaky) = flaky_uploads(&f);
    let s = f.sessions.create("").await.unwrap();
    let st = svc.init(&s.code, new_upload("a.txt", 3)).await.unwrap();
    svc.put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await
        .unwrap();
    *flaky.fail_next_add.lock().unwrap() = Some(RepoError::Storage("connection reset".into()));
    assert!(svc.complete(&s.code, st.upload_id).await.is_err());
    assert_eq!(f.blobs.keys().len(), 1);

    // Nobody retried within a day.
    f.clock.advance(chrono::Duration::hours(25));
    svc.cleanup_stale(chrono::Duration::hours(24))
        .await
        .unwrap();
    assert!(f.blobs.keys().is_empty());
    assert!(f
        .uploads
        .uploads
        .for_session(&s.code)
        .await
        .unwrap()
        .is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}
