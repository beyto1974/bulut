//! The per-session change log: written inside the transaction of each change, read by cursor.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::domain::change::{Change, ChangeKind, ChangePage};
use crate::ports::change_repo::ChangeRepo;
use crate::ports::session_repo::RepoError;

/// Entries kept per session. A client further behind lists the session again.
pub const KEEP: i64 = 1000;

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

/// What a change is about.
#[derive(Default)]
pub struct Subject<'a> {
    pub node_id: Option<Uuid>,
    pub version_id: Option<Uuid>,
    pub name: Option<&'a str>,
    pub version: Option<i32>,
    pub tag: Option<&'a str>,
}

/// Takes the session row before any node row. Every write to a session's tree starts here, so
/// they all lock in the same order as deleting the session (session row, then its nodes) and
/// cannot deadlock with it. Held until the transaction ends.
pub async fn lock_session(
    tx: &mut Transaction<'_, Postgres>,
    session: &str,
) -> Result<(), RepoError> {
    sqlx::query("SELECT 1 FROM sessions WHERE code = $1 FOR NO KEY UPDATE")
        .bind(session)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage)?
        .map(|_| ())
        .ok_or(RepoError::NotFound)
}

/// Appends to the log of `session` inside `tx`. The counter update locks the session row until
/// the transaction ends, so sequence numbers follow commit order and have no gaps.
pub async fn record(
    tx: &mut Transaction<'_, Postgres>,
    session: &str,
    kind: ChangeKind,
    subject: Subject<'_>,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let seq: i64 = sqlx::query(
        "UPDATE sessions SET change_seq = change_seq + 1 WHERE code = $1 RETURNING change_seq",
    )
    .bind(session)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(RepoError::NotFound)?
    .get("change_seq");
    sqlx::query(
        "INSERT INTO changes (session_code, seq, kind, node_id, version_id, name, version, tag, at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(session)
    .bind(seq)
    .bind(kind.as_str())
    .bind(subject.node_id)
    .bind(subject.version_id)
    .bind(subject.name)
    .bind(subject.version)
    .bind(subject.tag)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM changes WHERE session_code = $1 AND seq <= $2")
        .bind(session)
        .bind(seq - KEEP)
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    Ok(())
}

#[derive(Clone)]
pub struct PgChangeRepo {
    pool: PgPool,
}

impl PgChangeRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn change_from(row: &PgRow) -> Change {
    Change {
        seq: row.get("seq"),
        kind: ChangeKind::parse(row.get::<String, _>("kind").as_str())
            .unwrap_or(ChangeKind::FileUpdated),
        node_id: row.get("node_id"),
        version_id: row.get("version_id"),
        name: row.get("name"),
        version: row.get("version"),
        tag: row.get("tag"),
        at: row.get("at"),
    }
}

#[async_trait]
impl ChangeRepo for PgChangeRepo {
    async fn since(&self, session: &str, after: i64, limit: i64) -> Result<ChangePage, RepoError> {
        // One snapshot for the head, the oldest entry and the page: pruning or a new entry in
        // between must not make the three disagree.
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let head: i64 = sqlx::query("SELECT change_seq FROM sessions WHERE code = $1")
            .bind(session)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?
            .map(|r| r.get("change_seq"))
            .ok_or(RepoError::NotFound)?;
        let oldest: Option<i64> =
            sqlx::query("SELECT min(seq) AS oldest FROM changes WHERE session_code = $1")
                .bind(session)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?
                .get("oldest");
        let lost = match oldest {
            Some(oldest) => after.saturating_add(1) < oldest,
            None => after != head,
        };
        if after > head || lost {
            return Ok(ChangePage {
                seq: head,
                reset: true,
                events: Vec::new(),
            });
        }
        let rows = sqlx::query(
            "SELECT seq, kind, node_id, version_id, name, version, tag, at FROM changes
             WHERE session_code = $1 AND seq > $2 ORDER BY seq LIMIT $3",
        )
        .bind(session)
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
        let events: Vec<Change> = rows.iter().map(change_from).collect();
        Ok(ChangePage {
            seq: events.last().map_or(head.max(after), |e| e.seq),
            reset: false,
            events,
        })
    }

    async fn head(&self, session: &str) -> Result<i64, RepoError> {
        sqlx::query("SELECT change_seq FROM sessions WHERE code = $1")
            .bind(session)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .map(|r| r.get("change_seq"))
            .ok_or(RepoError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests (need `DATABASE_URL`).
    use super::*;
    use crate::adapters::postgres::{connect, PgSessionRepo};
    use crate::adapters::postgres_nodes::PgNodeRepo;
    use crate::domain::short_code::ShortCode;
    use crate::ports::code_generator::{CodeGenerator, RandomCodeGenerator};
    use crate::ports::node_repo::{NewVersion, NodePatch, NodeRepo};
    use crate::ports::session_repo::SessionRepo;

    struct Env {
        nodes: PgNodeRepo,
        sessions: PgSessionRepo,
        changes: PgChangeRepo,
        pool: PgPool,
        code: String,
    }

    async fn setup() -> Env {
        dotenvy::dotenv().ok();
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = connect(&url).await.unwrap();
        let sessions = PgSessionRepo::new(pool.clone());
        let code: ShortCode = RandomCodeGenerator::new(8).generate();
        sessions
            .create(code.as_str(), "", Utc::now())
            .await
            .unwrap();
        Env {
            nodes: PgNodeRepo::new(pool.clone()),
            changes: PgChangeRepo::new(pool.clone()),
            sessions,
            pool,
            code: code.as_str().to_string(),
        }
    }

    fn nv(key: &str) -> NewVersion {
        NewVersion {
            size: 1,
            content_type: "text/plain".into(),
            sha256: None,
            blob_key: key.into(),
            thumb_key: None,
            client_created_at: None,
        }
    }

    fn kinds(page: &ChangePage) -> Vec<ChangeKind> {
        page.events.iter().map(|e| e.kind).collect()
    }

    #[tokio::test]
    async fn a_new_session_has_an_empty_log() {
        let e = setup().await;
        let page = e.changes.since(&e.code, 0, 100).await.unwrap();
        assert_eq!((page.seq, page.reset, page.events.len()), (0, false, 0));
        assert_eq!(e.changes.head(&e.code).await.unwrap(), 0);
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn every_kind_of_change_is_logged_in_order() {
        let e = setup().await;
        let now = Utc::now();
        let (node, v1) = e
            .nodes
            .add_version(&e.code, None, "a.txt", nv("k1"), now)
            .await
            .unwrap();
        let (_, v2) = e
            .nodes
            .add_version(&e.code, None, "a.txt", nv("k2"), now)
            .await
            .unwrap();
        e.nodes.add_tag(&e.code, v1.id, "stable").await.unwrap();
        e.nodes.remove_tag(&e.code, v1.id, "stable").await.unwrap();
        e.nodes
            .update(
                &e.code,
                node.id,
                NodePatch {
                    name: Some("b.txt".into()),
                    note: None,
                },
            )
            .await
            .unwrap();
        e.sessions.set_description(&e.code, "hello").await.unwrap();
        e.nodes.delete(&e.code, node.id).await.unwrap();

        let page = e.changes.since(&e.code, 0, 100).await.unwrap();
        assert!(!page.reset);
        assert_eq!(
            kinds(&page),
            [
                ChangeKind::VersionAdded,
                ChangeKind::VersionAdded,
                ChangeKind::TagAdded,
                ChangeKind::TagRemoved,
                ChangeKind::FileUpdated,
                ChangeKind::SessionUpdated,
                ChangeKind::FileDeleted,
            ]
        );
        let seqs: Vec<i64> = page.events.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(page.seq, 7);
        let e1 = &page.events[0];
        assert_eq!(
            (e1.node_id, e1.version_id, e1.name.as_deref(), e1.version),
            (Some(node.id), Some(v1.id), Some("a.txt"), Some(1))
        );
        assert_eq!(page.events[1].version_id, Some(v2.id));
        assert_eq!(page.events[2].tag.as_deref(), Some("stable"));
        // The tombstone keeps what the tree no longer has.
        let gone = page.events.last().unwrap();
        assert_eq!(
            (gone.node_id, gone.name.as_deref()),
            (Some(node.id), Some("b.txt"))
        );

        // A cursor in the middle gets the rest only.
        let rest = e.changes.since(&e.code, 5, 100).await.unwrap();
        assert_eq!(
            kinds(&rest),
            [ChangeKind::SessionUpdated, ChangeKind::FileDeleted]
        );
        // At the head there is nothing, and the cursor stays.
        let none = e.changes.since(&e.code, 7, 100).await.unwrap();
        assert_eq!((none.seq, none.reset, none.events.len()), (7, false, 0));
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn limit_cuts_a_page_and_the_cursor_continues() {
        let e = setup().await;
        let now = Utc::now();
        for i in 0..3 {
            e.nodes
                .add_version(&e.code, None, &format!("f{i}"), nv(&format!("k{i}")), now)
                .await
                .unwrap();
        }
        let first = e.changes.since(&e.code, 0, 2).await.unwrap();
        assert_eq!((first.events.len(), first.seq), (2, 2));
        let second = e.changes.since(&e.code, first.seq, 2).await.unwrap();
        assert_eq!((second.events.len(), second.seq), (1, 3));
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn failed_changes_leave_no_entry() {
        let e = setup().await;
        let missing = Uuid::new_v4();
        assert!(e.nodes.delete(&e.code, missing).await.is_err());
        assert!(e.nodes.add_tag(&e.code, missing, "x").await.is_err());
        assert_eq!(e.changes.head(&e.code).await.unwrap(), 0);
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn an_old_or_future_cursor_asks_for_a_reset() {
        let e = setup().await;
        let mut tx = e.pool.begin().await.unwrap();
        for _ in 0..(KEEP + 5) {
            record(
                &mut tx,
                &e.code,
                ChangeKind::SessionUpdated,
                Subject::default(),
                Utc::now(),
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let head = e.changes.head(&e.code).await.unwrap();
        assert_eq!(head, KEEP + 5);
        // Entries 1..=5 were pruned: 0 is too old, 5 is just enough, a later one is a future cursor.
        let old = e.changes.since(&e.code, 0, 10).await.unwrap();
        assert!(old.reset && old.events.is_empty());
        assert_eq!(old.seq, head);
        let edge = e.changes.since(&e.code, 5, 10).await.unwrap();
        assert!(!edge.reset);
        assert_eq!(edge.events.first().map(|c| c.seq), Some(6));
        let future = e.changes.since(&e.code, head + 10, 10).await.unwrap();
        assert!(future.reset);
        assert_eq!(future.seq, head);
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn parallel_uploads_get_distinct_ordered_numbers() {
        let e = setup().await;
        let now = Utc::now();
        let mut tasks = Vec::new();
        for i in 0..8 {
            let nodes = e.nodes.clone();
            let code = e.code.clone();
            tasks.push(tokio::spawn(async move {
                nodes
                    .add_version(&code, None, &format!("p{i}"), nv(&format!("pk{i}")), now)
                    .await
                    .unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let page = e.changes.since(&e.code, 0, 100).await.unwrap();
        let seqs: Vec<i64> = page.events.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, (1..=8).collect::<Vec<_>>());
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_session_is_not_found() {
        let e = setup().await;
        assert!(matches!(
            e.changes.since("nope1234", 0, 10).await,
            Err(RepoError::NotFound)
        ));
        e.sessions.delete(&e.code).await.unwrap();
    }

    #[tokio::test]
    async fn deleting_the_session_removes_its_log() {
        let e = setup().await;
        e.nodes
            .add_version(&e.code, None, "a", nv("k"), Utc::now())
            .await
            .unwrap();
        e.sessions.delete(&e.code).await.unwrap();
        let n: i64 = sqlx::query("SELECT count(*) AS n FROM changes WHERE session_code = $1")
            .bind(&e.code)
            .fetch_one(&e.pool)
            .await
            .unwrap()
            .get("n");
        assert_eq!(n, 0);
    }
}
