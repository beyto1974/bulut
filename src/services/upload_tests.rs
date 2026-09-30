//! Upload service tests. The fixture uses 10 byte parts and a 1000 byte file limit.

use bytes::Bytes;
use chrono::Duration;
use futures_util::{stream, StreamExt};

use crate::ports::blob_store::BlobStore;
use crate::services::error::ServiceError;
use crate::services::tests::{fixture, Fixture};
use crate::services::upload_service::NewUpload;

fn new_upload(name: &str, size: i64) -> NewUpload {
    NewUpload {
        parent: None,
        name: name.into(),
        size,
        content_type: Some("text/plain".into()),
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

fn chunks(
    parts: &[&'static [u8]],
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Unpin + Send {
    stream::iter(
        parts
            .iter()
            .map(|p| Ok(Bytes::from_static(p)))
            .collect::<Vec<_>>(),
    )
}

#[tokio::test]
async fn chunked_upload_creates_a_file_and_a_second_one_adds_a_version() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();

    let st = f
        .uploads
        .init(&s.code, new_upload("a.txt", 25))
        .await
        .unwrap();
    assert_eq!((st.part_size, st.parts_total), (10, 3));
    f.uploads
        .put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"0123456789"))
        .await
        .unwrap();
    f.uploads
        .put_part(&s.code, st.upload_id, 3, Bytes::from_static(b"abcde"))
        .await
        .unwrap();
    // Parts can arrive out of order and be sent again.
    f.uploads
        .put_part(&s.code, st.upload_id, 2, Bytes::from_static(b"ABCDEFGHIJ"))
        .await
        .unwrap();
    f.uploads
        .put_part(&s.code, st.upload_id, 2, Bytes::from_static(b"ABCDEFGHIJ"))
        .await
        .unwrap();
    let (node, v1) = f.uploads.complete(&s.code, st.upload_id).await.unwrap();
    assert_eq!((v1.version, v1.size), (1, 25));
    assert_eq!(
        read_all(&f, &v1.blob_key).await,
        b"0123456789ABCDEFGHIJabcde"
    );
    assert_eq!(f.blobs.open_uploads(), 0);

    let st2 = f
        .uploads
        .init(&s.code, new_upload("a.txt", 3))
        .await
        .unwrap();
    f.uploads
        .put_part(&s.code, st2.upload_id, 1, Bytes::from_static(b"xyz"))
        .await
        .unwrap();
    let (node2, v2) = f.uploads.complete(&s.code, st2.upload_id).await.unwrap();
    assert_eq!((node2.id, v2.version), (node.id, 2));
    assert!(v2.tags.contains(&"latest".to_string()));
    assert_eq!(f.blobs.keys().len(), 2, "both versions are kept");
    f.sessions.delete(&s.code).await.unwrap();
    assert!(f.blobs.keys().is_empty());
}

#[tokio::test]
async fn resume_reports_the_parts_already_stored() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let st = f
        .uploads
        .init(&s.code, new_upload("r.bin", 25))
        .await
        .unwrap();
    assert!(f
        .uploads
        .status_of(&s.code, st.upload_id)
        .await
        .unwrap()
        .parts_received
        .is_empty());
    f.uploads
        .put_part(&s.code, st.upload_id, 2, Bytes::from_static(b"ABCDEFGHIJ"))
        .await
        .unwrap();
    let again = f.uploads.status_of(&s.code, st.upload_id).await.unwrap();
    assert_eq!(again.parts_received, [2]);
    assert!(matches!(
        f.uploads.complete(&s.code, st.upload_id).await,
        Err(ServiceError::Conflict(m)) if m.contains("missing")
    ));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn parts_are_validated() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let st = f
        .uploads
        .init(&s.code, new_upload("v.bin", 25))
        .await
        .unwrap();
    let id = st.upload_id;
    assert!(matches!(
        f.uploads
            .put_part(&s.code, id, 0, Bytes::from_static(b"0123456789"))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        f.uploads
            .put_part(&s.code, id, 4, Bytes::from_static(b"0123456789"))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    // A middle part must be full size, the last part must be exactly the remainder.
    assert!(matches!(
        f.uploads
            .put_part(&s.code, id, 1, Bytes::from_static(b"short"))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        f.uploads
            .put_part(&s.code, id, 3, Bytes::from_static(b"0123456789"))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    // Another session cannot touch this upload.
    let other = f.sessions.create("").await.unwrap();
    assert!(matches!(
        f.uploads
            .put_part(&other.code, id, 1, Bytes::from_static(b"0123456789"))
            .await,
        Err(ServiceError::NotFound)
    ));
    f.sessions.delete(&s.code).await.unwrap();
    f.sessions.delete(&other.code).await.unwrap();
}

#[tokio::test]
async fn init_rejects_bad_input() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    assert!(matches!(
        f.uploads.init(&s.code, new_upload("a/b", 1)).await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(
        matches!(f.uploads.init(&s.code, new_upload("big", 1001)).await, Err(ServiceError::Invalid(m)) if m.contains("limit"))
    );
    assert!(matches!(
        f.uploads.init(&s.code, new_upload("neg", -1)).await,
        Err(ServiceError::Invalid(_))
    ));
    let mut bad_parent = new_upload("x", 1);
    bad_parent.parent = Some(uuid::Uuid::new_v4());
    assert!(matches!(
        f.uploads.init(&s.code, bad_parent).await,
        Err(ServiceError::NotFound)
    ));
    assert_eq!(f.blobs.open_uploads(), 0, "nothing was started");
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn empty_files_are_supported() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let st = f
        .uploads
        .init(&s.code, new_upload("empty.txt", 0))
        .await
        .unwrap();
    assert_eq!(st.parts_total, 0);
    let (_, v) = f.uploads.complete(&s.code, st.upload_id).await.unwrap();
    assert_eq!(v.size, 0);
    assert!(read_all(&f, &v.blob_key).await.is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn completing_onto_a_folder_name_cleans_up() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    f.tree.create_folder(&s.code, None, "taken").await.unwrap();
    let st = f
        .uploads
        .init(&s.code, new_upload("taken", 3))
        .await
        .unwrap();
    f.uploads
        .put_part(&s.code, st.upload_id, 1, Bytes::from_static(b"abc"))
        .await
        .unwrap();
    assert!(matches!(
        f.uploads.complete(&s.code, st.upload_id).await,
        Err(ServiceError::Conflict(_))
    ));
    assert!(f.blobs.keys().is_empty(), "the stored object was removed");
    assert!(matches!(
        f.uploads.status_of(&s.code, st.upload_id).await,
        Err(ServiceError::NotFound)
    ));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn abort_and_session_purge_release_open_uploads() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let a = f.uploads.init(&s.code, new_upload("a", 25)).await.unwrap();
    f.uploads.init(&s.code, new_upload("b", 25)).await.unwrap();
    assert_eq!(f.blobs.open_uploads(), 2);
    f.uploads.abort(&s.code, a.upload_id).await.unwrap();
    assert_eq!(f.blobs.open_uploads(), 1);
    f.sessions.delete(&s.code).await.unwrap();
    assert_eq!(
        f.blobs.open_uploads(),
        0,
        "purging the session aborts what is left"
    );
}

#[tokio::test]
async fn abandoned_uploads_are_cleaned_up() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    f.uploads
        .init(&s.code, new_upload("old", 25))
        .await
        .unwrap();
    f.clock.advance(Duration::hours(30));
    assert_eq!(
        f.uploads.cleanup_stale(Duration::hours(24)).await.unwrap(),
        1
    );
    assert_eq!(f.blobs.open_uploads(), 0);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn streamed_upload_is_cut_into_parts_and_tagged() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    // 25 bytes arriving in uneven pieces become parts of 10, 10 and 5 bytes.
    let body = chunks(&[b"0123456", b"789ABCD", b"EFGHIJK", b"LMNO"]);
    let tags = vec!["v1.4.2".to_string()];
    let (node, v) = f
        .uploads
        .upload_stream(&s.code, None, "build.apk", None, None, &tags, body)
        .await
        .unwrap();
    assert_eq!(
        (node.name.as_str(), v.size, v.version),
        ("build.apk", 25, 1)
    );
    assert_eq!(v.content_type, "application/octet-stream");
    assert!(v.tags.contains(&"v1.4.2".to_string()));
    assert_eq!(
        read_all(&f, &v.blob_key).await,
        b"0123456789ABCDEFGHIJKLMNO"
    );
    assert_eq!(f.blobs.open_uploads(), 0);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn streamed_upload_edge_cases() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    // Exact multiple of the part size.
    let (_, v) = f
        .uploads
        .upload_stream(
            &s.code,
            None,
            "exact",
            None,
            None,
            &[],
            chunks(&[b"0123456789", b"ABCDEFGHIJ"]),
        )
        .await
        .unwrap();
    assert_eq!(v.size, 20);
    // Empty body.
    let (_, v) = f
        .uploads
        .upload_stream(&s.code, None, "empty", None, None, &[], chunks(&[]))
        .await
        .unwrap();
    assert_eq!(v.size, 0);
    // Over the limit: nothing is left behind.
    let big: &'static [u8] = &[1u8; 600];
    let err = f
        .uploads
        .upload_stream(&s.code, None, "big", None, None, &[], chunks(&[big, big]))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Invalid(m) if m.contains("limit")));
    assert_eq!(f.blobs.open_uploads(), 0);
    assert_eq!(f.blobs.keys().len(), 2);
    // Bad tag is rejected before anything is stored.
    let tags = vec!["two words".to_string()];
    assert!(matches!(
        f.uploads
            .upload_stream(&s.code, None, "t", None, None, &tags, chunks(&[b"x"]))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    assert_eq!(f.blobs.open_uploads(), 0);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_dropped_streamed_upload_releases_its_multipart_upload() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let uploads = f.uploads.clone();
    let code = s.code.clone();
    // One part arrives, then the client goes quiet and the connection is cut.
    let body =
        stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"0123456789abc")) })
            .chain(stream::pending::<Result<Bytes, std::io::Error>>());
    let task = tokio::spawn(async move {
        uploads
            .upload_stream(&code, None, "cut.bin", None, None, &[], Box::pin(body))
            .await
    });
    for _ in 0..200 {
        if f.blobs.open_uploads() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(f.blobs.open_uploads(), 1, "the upload was under way");
    task.abort();
    let _ = task.await;
    for _ in 0..200 {
        if f.blobs.open_uploads() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(f.blobs.open_uploads(), 0, "the dropped upload was aborted");
    assert!(f.blobs.keys().is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}
