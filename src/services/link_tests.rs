//! Upload link tests. The fixture allows 3 files per link and keeps a link for 15 minutes.

use bytes::Bytes;
use chrono::Duration;
use futures_util::stream;

use crate::services::error::ServiceError;
use crate::services::tests::fixture;

fn body(
    text: &'static [u8],
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Unpin + Send {
    stream::iter(vec![Ok(Bytes::from_static(text))])
}

#[tokio::test]
async fn a_link_takes_its_number_of_files_then_refuses() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let link = f.links.create(&s.code, Some(2)).await.unwrap();
    assert_eq!(link.files_total, 2);

    for name in ["a.txt", "b.txt"] {
        f.links
            .upload(&link.id, name, None, &[], body(b"hi"))
            .await
            .unwrap();
    }
    assert!(matches!(
        f.links
            .upload(&link.id, "c.txt", None, &[], body(b"hi"))
            .await,
        Err(ServiceError::NotFound)
    ));
    assert_eq!(f.tree.list(&s.code, None).await.unwrap().len(), 2);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_link_defaults_to_one_file_and_respects_the_limit() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    assert_eq!(f.links.create(&s.code, None).await.unwrap().files_total, 1);
    assert_eq!(
        f.links.create(&s.code, Some(3)).await.unwrap().files_total,
        3
    );
    for bad in [0, 4] {
        assert!(matches!(
            f.links.create(&s.code, Some(bad)).await,
            Err(ServiceError::Invalid(_))
        ));
    }
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn an_expired_link_is_refused() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let link = f.links.create(&s.code, None).await.unwrap();
    f.clock.advance(Duration::minutes(16));
    assert!(matches!(
        f.links
            .upload(&link.id, "a.txt", None, &[], body(b"hi"))
            .await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn a_failed_upload_does_not_use_up_the_link() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let link = f.links.create(&s.code, None).await.unwrap();
    assert!(f
        .links
        .upload(&link.id, "bad/name", None, &[], body(b"hi"))
        .await
        .is_err());
    f.links
        .upload(&link.id, "ok.txt", None, &["v1".into()], body(b"hi"))
        .await
        .unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn unknown_link_and_unknown_session_are_not_found() {
    let f = fixture().await;
    assert!(matches!(
        f.links
            .upload("nope", "a.txt", None, &[], body(b"hi"))
            .await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        f.links.create("zzzzz", None).await,
        Err(ServiceError::NotFound)
    ));
}
