//! Limits that keep one session from filling the storage: unfinished uploads and tags per version
//! (the versions and bytes limits live in the repository and are tested there).

use bytes::Bytes;
use futures_util::stream;

use crate::ports::clock::Clock;
use crate::ports::node_repo::{NewVersion, NodeRepo};
use crate::services::error::ServiceError;
use crate::services::limit_tests::limited_uploads;
use crate::services::tests::fixture;
use crate::services::tree_service::TreeService;
use crate::services::upload_service::{NewUpload, UploadService};

fn new_upload(name: &str) -> NewUpload {
    NewUpload {
        parent: None,
        name: name.into(),
        size: 25,
        content_type: None,
        client_created_at: None,
    }
}

#[tokio::test]
async fn too_many_unfinished_uploads_in_one_session_are_refused() {
    let f = fixture().await;
    let svc = UploadService {
        max_pending: 2,
        ..limited_uploads(&f, 100).await
    };
    let s = f.sessions.create("").await.unwrap();
    let a = svc.init(&s.code, new_upload("a")).await.unwrap();
    svc.init(&s.code, new_upload("b")).await.unwrap();

    let err = svc.init(&s.code, new_upload("c")).await.unwrap_err();
    assert!(
        matches!(&err, ServiceError::Conflict(m) if m.contains("2 unfinished uploads")),
        "{err}"
    );
    assert_eq!(
        f.blobs.open_uploads(),
        2,
        "the refused upload was not started"
    );

    // Cancelling one frees a place, and another session is not affected at all.
    svc.abort(&s.code, a.upload_id).await.unwrap();
    svc.init(&s.code, new_upload("c")).await.unwrap();
    let other = f.sessions.create("").await.unwrap();
    svc.init(&other.code, new_upload("a")).await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
    f.sessions.delete(&other.code).await.unwrap();
}

#[tokio::test]
async fn tags_per_version_are_limited() {
    let f = fixture().await;
    let tree = TreeService {
        folders_enabled: true,
        max_tags: 2,
        nodes: f.tree.nodes.clone(),
        blobs: f.tree.blobs.clone(),
        clock: f.tree.clock.clone(),
    };
    let s = f.sessions.create("").await.unwrap();
    let new = NewVersion {
        size: 1,
        content_type: "text/plain".into(),
        sha256: None,
        blob_key: "k".into(),
        thumb_key: None,
        client_created_at: None,
    };
    let (_, v) = f
        .nodes
        .add_version(&s.code, None, "a.txt", new, f.clock.now())
        .await
        .unwrap();

    tree.add_tag(&s.code, v.id, "one").await.unwrap();
    tree.add_tag(&s.code, v.id, "two").await.unwrap();
    let err = tree.add_tag(&s.code, v.id, "three").await.unwrap_err();
    assert!(
        matches!(&err, ServiceError::Conflict(m) if m.contains("at most 2 tags")),
        "{err}"
    );
    // A tag the version already has is not a new tag, the automatic `latest` does not count, and
    // removing one makes room.
    tree.add_tag(&s.code, v.id, "one").await.unwrap();
    tree.remove_tag(&s.code, v.id, "one").await.unwrap();
    tree.add_tag(&s.code, v.id, "three").await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_streamed_upload_with_too_many_tags_is_refused_before_any_bytes() {
    let f = fixture().await;
    let svc = UploadService {
        max_tags: 2,
        ..limited_uploads(&f, 100).await
    };
    let s = f.sessions.create("").await.unwrap();
    let tags: Vec<String> = ["a", "b", "c"].iter().map(|t| t.to_string()).collect();
    let body = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(b"x"))]);
    let err = svc
        .upload_stream(&s.code, None, "f", None, None, &tags, body)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ServiceError::Invalid(m) if m.contains("at most 2 tags")),
        "{err}"
    );
    assert_eq!(f.blobs.open_uploads(), 0);
    assert!(f.blobs.keys().is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}
