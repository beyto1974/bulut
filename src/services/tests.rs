//! Service tests with the real Postgres repositories (needs `DATABASE_URL`), an in-memory blob
//! store and a clock the test controls.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{Duration, Utc};

use crate::adapters::memory_blobs::MemoryBlobStore;
use crate::adapters::postgres::{connect, PgSessionRepo};
use crate::adapters::postgres_nodes::PgNodeRepo;
use crate::adapters::postgres_uploads::PgUploadRepo;
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::{Clock, ManualClock};
use crate::ports::code_generator::RandomCodeGenerator;
use crate::ports::node_repo::{NewVersion, NodeRepo};
use crate::services::error::ServiceError;
use crate::services::session_service::SessionService;
use crate::services::tree_service::TreeService;
use crate::services::upload_service::UploadService;

// The sweeper works on the whole database, so tests that move the clock must not overlap.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct Fixture {
    _guard: tokio::sync::MutexGuard<'static, ()>,
    pub sessions: Arc<SessionService>,
    pub tree: Arc<TreeService>,
    pub uploads: Arc<UploadService>,
    pub nodes: Arc<PgNodeRepo>,
    pub blobs: Arc<MemoryBlobStore>,
    pub clock: Arc<ManualClock>,
}

pub async fn fixture() -> Fixture {
    let guard = LOCK.lock().await;
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = connect(&url).await.unwrap();
    let blobs = Arc::new(MemoryBlobStore::new());
    let nodes = Arc::new(PgNodeRepo::new(pool.clone()));
    // Start far in the past so a sweep here never reaches sessions other tests made with the real clock.
    let clock = Arc::new(ManualClock::new(Utc::now() - Duration::days(120)));
    let uploads = Arc::new(UploadService {
        uploads: Arc::new(PgUploadRepo::new(pool.clone())),
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        clock: clock.clone(),
        // Small parts keep tests fast; the memory store has no 5 MiB minimum.
        part_size: 10,
        max_file_bytes: 1000,
    });
    let sessions = Arc::new(SessionService {
        sessions: Arc::new(PgSessionRepo::new(pool)),
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        uploads: uploads.clone(),
        codes: Arc::new(RandomCodeGenerator::new(5)),
        clock: clock.clone(),
        code_length: 5,
        idle_ttl_days: 7,
    });
    let tree = Arc::new(TreeService {
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        clock: clock.clone(),
    });
    Fixture {
        _guard: guard,
        sessions,
        tree,
        uploads,
        nodes,
        blobs,
        clock,
    }
}

fn nv(key: &str) -> NewVersion {
    NewVersion {
        size: 3,
        content_type: "text/plain".into(),
        sha256: None,
        blob_key: key.into(),
        thumb_key: None,
        client_created_at: None,
    }
}

#[tokio::test]
async fn create_then_open_returns_the_session() {
    let f = fixture().await;
    let s = f.sessions.create("  hello ").await.unwrap();
    assert_eq!(s.code.len(), 5);
    assert_eq!(s.description, "hello");
    assert_eq!(
        f.sessions.open(&s.code.to_uppercase()).await.unwrap().code,
        s.code
    );
    f.sessions.delete(&s.code).await.unwrap();
    assert!(matches!(
        f.sessions.open(&s.code).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn bad_codes_are_not_found_not_errors() {
    let f = fixture().await;
    for bad in ["", "abc", "k7m3o", "toolongcode", "k7m3-"] {
        assert!(
            matches!(f.sessions.open(bad).await, Err(ServiceError::NotFound)),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn opening_keeps_a_session_alive_past_the_window() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    // Day 6: someone views the session.
    f.clock.advance(Duration::days(6));
    f.sessions.open(&s.code).await.unwrap();
    // Day 10 is 4 days after the last view, so it survives the sweep.
    f.clock.advance(Duration::days(4));
    assert!(!f.sessions.sweep_idle().await.unwrap().contains(&s.code));
    // Day 13 is 7 days after the last view, so it is purged.
    f.clock.advance(Duration::days(3));
    assert!(f.sessions.sweep_idle().await.unwrap().contains(&s.code));
    assert!(matches!(
        f.sessions.open(&s.code).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn sweep_removes_idle_sessions_and_their_objects_only() {
    let f = fixture().await;
    let idle = f.sessions.create("").await.unwrap();
    f.nodes
        .add_version(&idle.code, None, "a.txt", nv("idle/a"), f.clock.now())
        .await
        .unwrap();
    f.blobs
        .put("idle/a", Bytes::from_static(b"abc"), "text/plain")
        .await
        .unwrap();
    f.clock.advance(Duration::days(8));
    let fresh = f.sessions.create("").await.unwrap();
    f.nodes
        .add_version(&fresh.code, None, "b.txt", nv("fresh/b"), f.clock.now())
        .await
        .unwrap();
    f.blobs
        .put("fresh/b", Bytes::from_static(b"abc"), "text/plain")
        .await
        .unwrap();

    let purged = f.sessions.sweep_idle().await.unwrap();
    assert!(purged.contains(&idle.code));
    assert!(!purged.contains(&fresh.code));
    assert_eq!(f.blobs.keys(), ["fresh/b"]);
    f.sessions.delete(&fresh.code).await.unwrap();
}

#[tokio::test]
async fn description_is_validated_and_saved() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let updated = f
        .sessions
        .set_description(&s.code, " notes for the reviewer ")
        .await
        .unwrap();
    assert_eq!(updated.description, "notes for the reviewer");
    let too_long = "x".repeat(2001);
    assert!(matches!(
        f.sessions.set_description(&s.code, &too_long).await,
        Err(ServiceError::Invalid(_))
    ));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn tree_operations_validate_input() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let dir = f
        .tree
        .create_folder(&s.code, None, " fixtures ")
        .await
        .unwrap();
    assert_eq!(dir.name, "fixtures");
    assert!(matches!(
        f.tree.create_folder(&s.code, None, "a/b").await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        f.tree.create_folder(&s.code, None, "fixtures").await,
        Err(ServiceError::Conflict(_))
    ));
    let sub = f
        .tree
        .create_folder(&s.code, Some(dir.id), "deep")
        .await
        .unwrap();
    let path = f.tree.path(&s.code, Some(sub.id)).await.unwrap();
    assert_eq!(
        path.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        ["fixtures", "deep"]
    );
    assert!(f.tree.path(&s.code, None).await.unwrap().is_empty());
    let updated = f
        .tree
        .update(&s.code, dir.id, None, Some("  only failing ones "))
        .await
        .unwrap();
    assert_eq!(updated.note, "only failing ones");
    assert!(matches!(
        f.tree
            .update(&s.code, dir.id, None, Some(&"n".repeat(4001)))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        f.tree.list(&s.code, Some(uuid::Uuid::new_v4())).await,
        Err(ServiceError::NotFound)
    ));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn deleting_a_node_removes_its_objects() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let dir = f.tree.create_folder(&s.code, None, "d").await.unwrap();
    f.nodes
        .add_version(&s.code, Some(dir.id), "x", nv("kx"), f.clock.now())
        .await
        .unwrap();
    f.blobs
        .put("kx", Bytes::from_static(b"abc"), "x")
        .await
        .unwrap();
    f.tree.delete(&s.code, dir.id).await.unwrap();
    assert!(f.blobs.keys().is_empty());
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn tags_are_validated() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let (_, v) = f
        .nodes
        .add_version(&s.code, None, "build.apk", nv("k"), f.clock.now())
        .await
        .unwrap();
    let tagged = f.tree.add_tag(&s.code, v.id, " v1.4.2 ").await.unwrap();
    assert!(tagged.tags.contains(&"v1.4.2".to_string()));
    assert!(matches!(
        f.tree.add_tag(&s.code, v.id, "two words").await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        f.tree.add_tag(&s.code, v.id, "latest").await,
        Err(ServiceError::Invalid(_))
    ));
    f.tree.remove_tag(&s.code, v.id, "v1.4.2").await.unwrap();
    f.sessions.delete(&s.code).await.unwrap();
}
