//! The per-session file limit: refused early, enforced atomically, and versions do not count.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;

use crate::adapters::postgres::connect;
use crate::adapters::postgres_nodes::PgNodeRepo;
use crate::services::error::ServiceError;
use crate::services::tests::{fixture, Fixture};
use crate::services::upload_service::{NewUpload, UploadService};

/// An upload service whose session limit is `max` files, sharing the fixture's storage and clock.
pub(crate) async fn limited_uploads(f: &Fixture, max: u32) -> UploadService {
    let pool = connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    UploadService {
        folders_enabled: true,
        uploads: f.uploads.uploads.clone(),
        nodes: Arc::new(PgNodeRepo::new(pool).with_max_files(i64::from(max))),
        blobs: f.blobs.clone(),
        clock: f.clock.clone(),
        part_size: 10,
        max_file_bytes: 1000,
        max_files: max,
    }
}

fn new_upload(name: &str) -> NewUpload {
    NewUpload {
        parent: None,
        name: name.into(),
        size: 3,
        content_type: None,
        client_created_at: None,
    }
}

async fn store(svc: &UploadService, code: &str, name: &str) -> Result<(), ServiceError> {
    let st = svc.init(code, new_upload(name)).await?;
    svc.put_part(code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await?;
    svc.complete(code, st.upload_id).await?;
    Ok(())
}

#[tokio::test]
async fn a_full_session_refuses_a_new_name_before_any_bytes_are_sent() {
    let f = fixture().await;
    let svc = limited_uploads(&f, 2).await;
    let s = f.sessions.create("").await.unwrap();
    store(&svc, &s.code, "a.txt").await.unwrap();
    store(&svc, &s.code, "b.txt").await.unwrap();

    let err = svc.init(&s.code, new_upload("c.txt")).await.unwrap_err();
    assert!(
        matches!(&err, ServiceError::Conflict(m) if m.contains("maximum of 2 files")),
        "{err}"
    );
    assert_eq!(
        f.blobs.open_uploads(),
        0,
        "no upload was started for the refused file"
    );

    // The streamed path refuses early as well, without touching the store.
    let body = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(b"abc"))]);
    let err = svc
        .upload_stream(&s.code, None, "c.txt", None, None, &[], body)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Conflict(_)));
    assert_eq!(f.blobs.open_uploads(), 0);
    assert_eq!(f.blobs.keys().len(), 2);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn new_versions_of_existing_files_do_not_count() {
    let f = fixture().await;
    let svc = limited_uploads(&f, 2).await;
    let s = f.sessions.create("").await.unwrap();
    store(&svc, &s.code, "a.txt").await.unwrap();
    store(&svc, &s.code, "b.txt").await.unwrap();
    // The session is full, but a third, fourth and fifth version of a.txt still fit.
    for _ in 0..3 {
        store(&svc, &s.code, "a.txt").await.unwrap();
    }
    assert_eq!(f.tree.list(&s.code, None).await.unwrap().len(), 2);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn deleting_a_file_makes_room() {
    let f = fixture().await;
    let svc = limited_uploads(&f, 1).await;
    let s = f.sessions.create("").await.unwrap();
    store(&svc, &s.code, "a.txt").await.unwrap();
    assert!(store(&svc, &s.code, "b.txt").await.is_err());
    let id = f.tree.list(&s.code, None).await.unwrap()[0].node.id;
    f.tree.delete(&s.code, id).await.unwrap();
    store(&svc, &s.code, "b.txt").await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn two_uploads_racing_for_the_last_slot_cannot_both_win() {
    let f = fixture().await;
    let svc = limited_uploads(&f, 1).await;
    let s = f.sessions.create("").await.unwrap();
    // Both pass the early check, because the session is still empty.
    let a = svc.init(&s.code, new_upload("a.txt")).await.unwrap();
    let b = svc.init(&s.code, new_upload("b.txt")).await.unwrap();
    for st in [&a, &b] {
        svc.put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
            .await
            .unwrap();
    }
    svc.complete(&s.code, a.upload_id).await.unwrap();
    // The repository has the last word, and the stored object of the loser is removed.
    let err = svc.complete(&s.code, b.upload_id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Conflict(m) if m.contains("maximum of 1 files")));
    assert_eq!(f.blobs.keys().len(), 1, "only the winner's object is kept");
    f.sessions.delete(&s.code).await.unwrap();
}
