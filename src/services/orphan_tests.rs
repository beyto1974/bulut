//! The orphan sweep with the real repositories and an in-memory store whose clock the test moves.

use std::sync::Arc;

use bytes::Bytes;
use chrono::Duration;
use futures_util::stream;

use crate::ports::blob_store::BlobStore;
use crate::ports::session_repo::RepoError;
use crate::services::complete_retry_tests::{flaky_uploads, new_upload};
use crate::services::orphan_service::OrphanSweeper;
use crate::services::tests::{fixture, Fixture};

fn sweeper(f: &Fixture) -> OrphanSweeper {
    OrphanSweeper {
        blobs: f.blobs.clone(),
        nodes: f.nodes.clone(),
        uploads: f.uploads.uploads.clone(),
        clock: f.clock.clone(),
        grace: Duration::hours(24),
        code_length: 5,
    }
}

async fn put(f: &Fixture, key: &str) {
    f.blobs
        .put(key, Bytes::from_static(b"x"), "text/plain")
        .await
        .unwrap();
}

#[tokio::test]
async fn unreferenced_old_objects_go_and_everything_else_stays() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let kept = f
        .uploads
        .upload_stream(
            &s.code,
            None,
            "kept.txt",
            None,
            None,
            &[],
            stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(b"hello"))]),
        )
        .await
        .unwrap()
        .1;

    let orphan = format!("{}/{}", s.code, uuid::Uuid::new_v4());
    // A purged session's leftover: nothing in the database knows its code any more.
    let from_purged_session = format!("zzzzz/{}", uuid::Uuid::new_v4());
    let foreign = "backups/2026/db.dump".to_string();
    put(&f, &orphan).await;
    put(&f, &from_purged_session).await;
    put(&f, &foreign).await;

    // Inside the grace period nothing is touched.
    f.clock.advance(Duration::hours(23));
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 0);

    // An orphan made now is protected while the older ones are not.
    let fresh = format!("{}/{}", s.code, uuid::Uuid::new_v4());
    put(&f, &fresh).await;
    f.clock.advance(Duration::hours(2));
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 2);

    let left = f.blobs.keys();
    assert!(left.contains(&kept.blob_key), "a referenced file stays");
    assert!(
        left.contains(&fresh),
        "an object inside the grace period stays"
    );
    assert!(left.contains(&foreign), "foreign keys are never touched");
    assert!(!left.contains(&orphan) && !left.contains(&from_purged_session));

    // Once it is old enough the fresh one goes too, and a second sweep finds nothing more.
    f.clock.advance(Duration::hours(24));
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 1);
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 0);
    let mut expected = vec![foreign, kept.blob_key.clone()];
    expected.sort();
    assert_eq!(f.blobs.keys(), expected);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn an_object_kept_for_a_retried_complete_is_not_an_orphan() {
    let f = fixture().await;
    let (svc, flaky) = flaky_uploads(&f);
    let s = f.sessions.create("").await.unwrap();
    let st = svc.init(&s.code, new_upload("a.txt", 3)).await.unwrap();
    svc.put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await
        .unwrap();
    *flaky.fail_next_add.lock().unwrap() = Some(RepoError::Storage("connection reset".into()));
    assert!(svc.complete(&s.code, st.upload_id).await.is_err());

    // Long past the grace period, but the upload row still points at the object.
    f.clock.advance(Duration::hours(30));
    let before = f.blobs.keys();
    assert_eq!(before.len(), 1);
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 0);
    assert_eq!(f.blobs.keys(), before);
    svc.complete(&s.code, st.upload_id).await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn sweeping_pages_through_a_bucket_larger_than_one_page() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let blobs: Arc<dyn BlobStore> = f.blobs.clone();
    for i in 0..2300 {
        blobs
            .put(
                &format!("{}/{i:05}", s.code),
                Bytes::from_static(b"x"),
                "text/plain",
            )
            .await
            .unwrap();
    }
    f.clock.advance(Duration::hours(30));
    assert_eq!(sweeper(&f).sweep().await.unwrap(), 2300);
    assert!(f.blobs.keys().is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}
