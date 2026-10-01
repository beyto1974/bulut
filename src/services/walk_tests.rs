//! The walk behind `llms.txt` and the MCP `get_session` tool.

use crate::ports::clock::Clock;
use crate::ports::node_repo::{NewVersion, NodeRepo};
use crate::services::tests::fixture;

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
async fn walk_carries_every_versions_newest_first_and_stops_at_the_limit() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let now = f.clock.now();
    for (name, key) in [("a", "k-a1"), ("a", "k-a2"), ("b", "k-b1"), ("c", "k-c1")] {
        f.nodes
            .add_version(&s.code, None, name, nv(key), now)
            .await
            .unwrap();
    }

    let walk = f.tree.walk(&s.code, 10).await.unwrap();
    assert!(!walk.truncated);
    let seen: Vec<(&str, Vec<i32>)> = walk
        .items
        .iter()
        .map(|i| {
            (
                i.path.as_str(),
                i.versions.iter().map(|v| v.version).collect(),
            )
        })
        .collect();
    assert_eq!(seen, [("a", vec![2, 1]), ("b", vec![1]), ("c", vec![1])]);

    let cut = f.tree.walk(&s.code, 2).await.unwrap();
    assert!(cut.truncated);
    assert_eq!(cut.items.len(), 2);
    assert!(cut.items.iter().all(|i| !i.versions.is_empty()));
    f.sessions.delete(&s.code).await.unwrap();
}
