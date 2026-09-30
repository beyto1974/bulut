//! Folders are built but switched off by default (`FOLDERS_ENABLED=false`).

use bytes::Bytes;
use futures_util::stream;

use crate::services::error::ServiceError;
use crate::services::tests::fixture;
use crate::services::tree_service::TreeService;
use crate::services::upload_service::{NewUpload, UploadService};

#[tokio::test]
async fn folders_are_refused_when_disabled_but_root_files_still_work() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    // A folder made while the feature was on, to try to upload into.
    let dir = f.tree.create_folder(&s.code, None, "old").await.unwrap();

    let tree = TreeService {
        folders_enabled: false,
        max_tags: 1000,
        nodes: f.tree.nodes.clone(),
        blobs: f.tree.blobs.clone(),
        clock: f.tree.clock.clone(),
    };
    let uploads = UploadService {
        folders_enabled: false,
        uploads: f.uploads.uploads.clone(),
        nodes: f.uploads.nodes.clone(),
        blobs: f.uploads.blobs.clone(),
        clock: f.uploads.clock.clone(),
        part_size: 10,
        max_file_bytes: 1000,
        max_files: 1000,
        max_pending: 1000,
        max_tags: 1000,
    };

    assert!(matches!(
        tree.create_folder(&s.code, None, "new").await,
        Err(ServiceError::Invalid(m)) if m.contains("not enabled")
    ));
    let into_folder = NewUpload {
        parent: Some(dir.id),
        name: "a.txt".into(),
        size: 3,
        content_type: None,
        client_created_at: None,
    };
    assert!(
        matches!(uploads.init(&s.code, into_folder).await, Err(ServiceError::Invalid(m)) if m.contains("not enabled"))
    );
    let body = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(b"abc"))]);
    assert!(matches!(
        uploads.upload_stream(&s.code, Some(dir.id), "a.txt", None, None, &[], body).await,
        Err(ServiceError::Invalid(m)) if m.contains("not enabled")
    ));
    assert_eq!(f.blobs.open_uploads(), 0, "nothing was started");

    // Files at the session root are unaffected.
    let root = NewUpload {
        parent: None,
        name: "a.txt".into(),
        size: 3,
        content_type: None,
        client_created_at: None,
    };
    let st = uploads.init(&s.code, root).await.unwrap();
    uploads
        .put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await
        .unwrap();
    uploads.complete(&s.code, st.upload_id).await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}
