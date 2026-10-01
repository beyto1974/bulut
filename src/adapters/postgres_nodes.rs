//! Postgres implementation of the file tree repository.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::domain::node::{FileVersion, Node, NodeEntry, NodeKind};
use crate::ports::node_repo::{NewVersion, NodePatch, NodeRepo};
use crate::ports::session_repo::RepoError;

/// Virtual tag carried by the newest version of every file. It is never stored.
pub const LATEST_TAG: &str = "latest";

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

fn is_unique(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

/// What one session may hold. Checked when a version is added, under a lock per session, so
/// parallel uploads cannot overshoot.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Files (names). New versions of an existing file do not count.
    pub max_files: i64,
    pub max_versions_per_file: i64,
    /// Sum of the sizes of all stored versions.
    pub max_session_bytes: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: i64::MAX,
            max_versions_per_file: i64::MAX,
            max_session_bytes: i64::MAX,
        }
    }
}

#[derive(Clone)]
pub struct PgNodeRepo {
    pool: PgPool,
    limits: Limits,
}

impl PgNodeRepo {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            limits: Limits::default(),
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Most files one session may hold. New versions of an existing file do not count.
    pub fn with_max_files(mut self, max_files: i64) -> Self {
        self.limits.max_files = max_files;
        self
    }
}

const NODE_COLS: &str = "id, session_code, parent_id, kind, name, note, created_at";
const VERSION_SELECT: &str = "SELECT v.id, v.node_id, v.version, v.size, v.content_type, v.sha256,
        v.blob_key, v.thumb_key, v.client_created_at, v.uploaded_at, v.is_latest,
        COALESCE(array_agg(t.tag ORDER BY t.tag) FILTER (WHERE t.tag IS NOT NULL), '{}') AS tags
    FROM file_versions v LEFT JOIN tags t ON t.version_id = v.id";

fn node_from(row: &PgRow) -> Node {
    Node {
        id: row.get("id"),
        session_code: row.get("session_code"),
        parent_id: row.get("parent_id"),
        kind: NodeKind::parse(row.get::<String, _>("kind").as_str()).unwrap_or(NodeKind::File),
        name: row.get("name"),
        note: row.get("note"),
        created_at: row.get("created_at"),
    }
}

fn version_from(row: &PgRow) -> FileVersion {
    let is_latest: bool = row.get("is_latest");
    let mut tags: Vec<String> = row.get("tags");
    if is_latest {
        tags.insert(0, LATEST_TAG.to_string());
    }
    let thumb_key: Option<String> = row.get("thumb_key");
    FileVersion {
        id: row.get("id"),
        node_id: row.get("node_id"),
        version: row.get("version"),
        size: row.get("size"),
        content_type: row.get("content_type"),
        sha256: row.get("sha256"),
        blob_key: row.get("blob_key"),
        has_thumbnail: thumb_key.is_some(),
        thumb_key,
        client_created_at: row.get("client_created_at"),
        uploaded_at: row.get("uploaded_at"),
        is_latest,
        tags,
    }
}

async fn check_parent(
    tx: &mut Transaction<'_, Postgres>,
    session: &str,
    parent: Option<Uuid>,
) -> Result<(), RepoError> {
    let Some(parent) = parent else { return Ok(()) };
    let row = sqlx::query("SELECT kind FROM nodes WHERE id = $1 AND session_code = $2")
        .bind(parent)
        .bind(session)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage)?;
    match row {
        Some(r) if r.get::<String, _>("kind") == "folder" => Ok(()),
        Some(_) => Err(RepoError::Invalid("parent is not a folder".into())),
        None => Err(RepoError::NotFound),
    }
}

/// Result of one try at adding a version.
enum Attempt {
    Done(Box<(Node, FileVersion)>),
    LostRace,
}

impl PgNodeRepo {
    async fn try_add_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        new: &NewVersion,
        now: DateTime<Utc>,
    ) -> Result<Attempt, RepoError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        check_parent(&mut tx, session, parent).await?;

        // Every limit below is checked and used under this one lock per session, so parallel
        // uploads cannot push the session past them. The lock ends with the transaction.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(session)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;

        let existing = sqlx::query(&format!(
            "SELECT {NODE_COLS} FROM nodes
             WHERE session_code = $1 AND parent_id IS NOT DISTINCT FROM $2::uuid AND name = $3
             FOR UPDATE"
        ))
        .bind(session)
        .bind(parent)
        .bind(name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;

        let node = match existing {
            Some(row) => {
                let node = node_from(&row);
                if node.kind == NodeKind::Folder {
                    return Err(RepoError::NameTaken);
                }
                let versions: i64 =
                    sqlx::query("SELECT count(*) AS n FROM file_versions WHERE node_id = $1")
                        .bind(node.id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(storage)?
                        .get("n");
                if versions >= self.limits.max_versions_per_file {
                    return Err(RepoError::versions_limit(self.limits.max_versions_per_file));
                }
                node
            }
            None => {
                let files: i64 = sqlx::query(
                    "SELECT count(*) AS n FROM nodes WHERE session_code = $1 AND kind = 'file'",
                )
                .bind(session)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?
                .get("n");
                if files >= self.limits.max_files {
                    return Err(RepoError::files_limit(self.limits.max_files));
                }
                let inserted = sqlx::query(&format!(
                    "INSERT INTO nodes (id, session_code, parent_id, kind, name, created_at)
                     VALUES ($1, $2, $3, 'file', $4, $5) RETURNING {NODE_COLS}"
                ))
                .bind(Uuid::new_v4())
                .bind(session)
                .bind(parent)
                .bind(name)
                .bind(now)
                .fetch_one(&mut *tx)
                .await;
                match inserted {
                    Ok(row) => node_from(&row),
                    // Someone else created the name between our lookup and insert. Dropping the
                    // transaction rolls it back, the caller retries.
                    Err(e) if is_unique(&e) => return Ok(Attempt::LostRace),
                    Err(e) => return Err(storage(e)),
                }
            }
        };

        let used: i64 = sqlx::query(
            "SELECT COALESCE(SUM(v.size), 0)::bigint AS n FROM file_versions v
             JOIN nodes n ON n.id = v.node_id WHERE n.session_code = $1",
        )
        .bind(session)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?
        .get("n");
        if used.saturating_add(new.size) > self.limits.max_session_bytes {
            return Err(RepoError::bytes_limit(self.limits.max_session_bytes));
        }

        let next: i32 = sqlx::query(
            "SELECT COALESCE(MAX(version), 0) + 1 AS next FROM file_versions WHERE node_id = $1",
        )
        .bind(node.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?
        .get("next");
        sqlx::query("UPDATE file_versions SET is_latest = FALSE WHERE node_id = $1 AND is_latest")
            .bind(node.id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO file_versions (id, node_id, version, size, content_type, sha256, blob_key,
                                        thumb_key, client_created_at, uploaded_at, is_latest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, TRUE)",
        )
        .bind(id)
        .bind(node.id)
        .bind(next)
        .bind(new.size)
        .bind(&new.content_type)
        .bind(&new.sha256)
        .bind(&new.blob_key)
        .bind(&new.thumb_key)
        .bind(new.client_created_at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        let row = sqlx::query(&format!("{VERSION_SELECT} WHERE v.id = $1 GROUP BY v.id"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(Attempt::Done(Box::new((node, version_from(&row)))))
    }
}

#[async_trait]
impl NodeRepo for PgNodeRepo {
    async fn create_folder(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<Node, RepoError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        check_parent(&mut tx, session, parent).await?;
        let res = sqlx::query(&format!(
            "INSERT INTO nodes (id, session_code, parent_id, kind, name, created_at)
             VALUES ($1, $2, $3, 'folder', $4, $5) RETURNING {NODE_COLS}"
        ))
        .bind(Uuid::new_v4())
        .bind(session)
        .bind(parent)
        .bind(name)
        .bind(now)
        .fetch_one(&mut *tx)
        .await;
        let row = match res {
            Ok(r) => r,
            Err(e) if is_unique(&e) => return Err(RepoError::NameTaken),
            Err(e) => return Err(storage(e)),
        };
        tx.commit().await.map_err(storage)?;
        Ok(node_from(&row))
    }

    async fn add_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        new: NewVersion,
        now: DateTime<Utc>,
    ) -> Result<(Node, FileVersion), RepoError> {
        // Two uploads of a new name can both find no file and both try to create it. The loser
        // rolls back and tries again, and then finds the file the winner made.
        for _ in 0..5 {
            match self
                .try_add_version(session, parent, name, &new, now)
                .await?
            {
                Attempt::Done(pair) => return Ok(*pair),
                Attempt::LostRace => continue,
            }
        }
        Err(RepoError::Storage(
            "too many uploads of the same name at once".into(),
        ))
    }

    async fn get(&self, session: &str, id: Uuid) -> Result<Option<Node>, RepoError> {
        let row = sqlx::query(&format!(
            "SELECT {NODE_COLS} FROM nodes WHERE id = $1 AND session_code = $2"
        ))
        .bind(id)
        .bind(session)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.as_ref().map(node_from))
    }

    async fn list(&self, session: &str, parent: Option<Uuid>) -> Result<Vec<NodeEntry>, RepoError> {
        let rows = sqlx::query(
            "SELECT n.id, n.session_code, n.parent_id, n.kind, n.name, n.note, n.created_at,
                    (SELECT count(*) FROM file_versions v WHERE v.node_id = n.id) AS version_count,
                    (SELECT count(*) FROM nodes c WHERE c.parent_id = n.id) AS child_count
             FROM nodes n
             WHERE n.session_code = $1 AND n.parent_id IS NOT DISTINCT FROM $2::uuid
             ORDER BY (n.kind = 'folder') DESC, lower(n.name), n.name",
        )
        .bind(session)
        .bind(parent)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
        let latest_rows = sqlx::query(&format!(
            "{VERSION_SELECT} WHERE v.node_id = ANY($1) AND v.is_latest GROUP BY v.id"
        ))
        .bind(&ids)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let latest: Vec<FileVersion> = latest_rows.iter().map(version_from).collect();
        Ok(rows
            .iter()
            .map(|r| {
                let node = node_from(r);
                NodeEntry {
                    latest: latest.iter().find(|v| v.node_id == node.id).cloned(),
                    version_count: r.get("version_count"),
                    child_count: r.get("child_count"),
                    node,
                }
            })
            .collect())
    }

    async fn update(&self, session: &str, id: Uuid, patch: NodePatch) -> Result<Node, RepoError> {
        let res = sqlx::query(&format!(
            "UPDATE nodes SET name = COALESCE($3, name), note = COALESCE($4, note)
             WHERE id = $1 AND session_code = $2 RETURNING {NODE_COLS}"
        ))
        .bind(id)
        .bind(session)
        .bind(patch.name)
        .bind(patch.note)
        .fetch_optional(&self.pool)
        .await;
        match res {
            Ok(Some(row)) => Ok(node_from(&row)),
            Ok(None) => Err(RepoError::NotFound),
            Err(e) if is_unique(&e) => Err(RepoError::NameTaken),
            Err(e) => Err(storage(e)),
        }
    }

    async fn delete(&self, session: &str, id: Uuid) -> Result<Vec<String>, RepoError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let rows = sqlx::query(
            "WITH RECURSIVE sub AS (
                 SELECT id FROM nodes WHERE id = $1 AND session_code = $2
                 UNION ALL
                 SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
             )
             SELECT blob_key, thumb_key FROM file_versions WHERE node_id IN (SELECT id FROM sub)",
        )
        .bind(id)
        .bind(session)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
        let deleted = sqlx::query("DELETE FROM nodes WHERE id = $1 AND session_code = $2")
            .bind(id)
            .bind(session)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        if deleted.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }
        tx.commit().await.map_err(storage)?;
        Ok(keys_from(&rows))
    }

    async fn all_keys(&self, session: &str) -> Result<Vec<String>, RepoError> {
        let rows = sqlx::query(
            "SELECT v.blob_key, v.thumb_key FROM file_versions v
             JOIN nodes n ON n.id = v.node_id WHERE n.session_code = $1",
        )
        .bind(session)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(keys_from(&rows))
    }

    async fn versions(&self, session: &str, node_id: Uuid) -> Result<Vec<FileVersion>, RepoError> {
        let rows = sqlx::query(&format!(
            "{VERSION_SELECT} JOIN nodes n ON n.id = v.node_id
             WHERE v.node_id = $1 AND n.session_code = $2
             GROUP BY v.id ORDER BY v.version DESC"
        ))
        .bind(node_id)
        .bind(session)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(rows.iter().map(version_from).collect())
    }

    async fn versions_of(
        &self,
        session: &str,
        node_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<FileVersion>>, RepoError> {
        if node_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query(&format!(
            "{VERSION_SELECT} JOIN nodes n ON n.id = v.node_id
             WHERE v.node_id = ANY($1) AND n.session_code = $2
             GROUP BY v.id ORDER BY v.version DESC"
        ))
        .bind(node_ids)
        .bind(session)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let mut out: HashMap<Uuid, Vec<FileVersion>> = HashMap::new();
        for row in &rows {
            let v = version_from(row);
            out.entry(v.node_id).or_default().push(v);
        }
        Ok(out)
    }

    async fn version(
        &self,
        session: &str,
        version_id: Uuid,
    ) -> Result<Option<(Node, FileVersion)>, RepoError> {
        let row = sqlx::query(&format!(
            "{VERSION_SELECT} JOIN nodes n ON n.id = v.node_id
             WHERE v.id = $1 AND n.session_code = $2 GROUP BY v.id"
        ))
        .bind(version_id)
        .bind(session)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        let Some(row) = row else { return Ok(None) };
        let version = version_from(&row);
        let node = self
            .get(session, version.node_id)
            .await?
            .ok_or(RepoError::NotFound)?;
        Ok(Some((node, version)))
    }

    async fn add_tag(
        &self,
        session: &str,
        version_id: Uuid,
        tag: &str,
    ) -> Result<FileVersion, RepoError> {
        if tag == LATEST_TAG {
            return Err(RepoError::Invalid(format!(
                "{LATEST_TAG:?} is set automatically"
            )));
        }
        let Some((node, _)) = self.version(session, version_id).await? else {
            return Err(RepoError::NotFound);
        };
        sqlx::query(
            "INSERT INTO tags (node_id, version_id, tag) VALUES ($1, $2, $3)
             ON CONFLICT (node_id, tag) DO UPDATE SET version_id = EXCLUDED.version_id",
        )
        .bind(node.id)
        .bind(version_id)
        .bind(tag)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        self.version(session, version_id)
            .await?
            .map(|(_, v)| v)
            .ok_or(RepoError::NotFound)
    }

    async fn remove_tag(
        &self,
        session: &str,
        version_id: Uuid,
        tag: &str,
    ) -> Result<(), RepoError> {
        if self.version(session, version_id).await?.is_none() {
            return Err(RepoError::NotFound);
        }
        sqlx::query("DELETE FROM tags WHERE version_id = $1 AND tag = $2")
            .bind(version_id)
            .bind(tag)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn find_version(
        &self,
        session: &str,
        parent: Option<Uuid>,
        name: &str,
        tag: Option<&str>,
    ) -> Result<Option<(Node, FileVersion)>, RepoError> {
        let tag = tag.filter(|t| *t != LATEST_TAG);
        let sql = format!(
            "{VERSION_SELECT} JOIN nodes n ON n.id = v.node_id
             WHERE n.session_code = $1 AND n.parent_id IS NOT DISTINCT FROM $2::uuid AND n.name = $3
               AND ({cond})
             GROUP BY v.id",
            cond = if tag.is_some() {
                "EXISTS (SELECT 1 FROM tags x WHERE x.version_id = v.id AND x.tag = $4)"
            } else {
                "v.is_latest AND $4::text IS NULL"
            }
        );
        let row = sqlx::query(&sql)
            .bind(session)
            .bind(parent)
            .bind(name)
            .bind(tag)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        let Some(row) = row else { return Ok(None) };
        let version = version_from(&row);
        let node = self
            .get(session, version.node_id)
            .await?
            .ok_or(RepoError::NotFound)?;
        Ok(Some((node, version)))
    }

    async fn count_files(&self, session: &str) -> Result<i64, RepoError> {
        Ok(
            sqlx::query(
                "SELECT count(*) AS n FROM nodes WHERE session_code = $1 AND kind = 'file'",
            )
            .bind(session)
            .fetch_one(&self.pool)
            .await
            .map_err(storage)?
            .get("n"),
        )
    }

    async fn set_thumb_key(&self, version_id: Uuid, key: &str) -> Result<(), RepoError> {
        sqlx::query("UPDATE file_versions SET thumb_key = $2 WHERE id = $1")
            .bind(version_id)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

fn keys_from(rows: &[PgRow]) -> Vec<String> {
    let mut keys = Vec::new();
    for r in rows {
        keys.push(r.get::<String, _>("blob_key"));
        if let Some(t) = r.get::<Option<String>, _>("thumb_key") {
            keys.push(t);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    //! Integration tests against the shared dev Postgres (needs `DATABASE_URL`).
    use super::*;
    use crate::adapters::postgres::{connect, PgSessionRepo};
    use crate::domain::short_code::ShortCode;
    use crate::ports::code_generator::{CodeGenerator, RandomCodeGenerator};
    use crate::ports::session_repo::SessionRepo;

    async fn setup() -> (PgNodeRepo, PgSessionRepo, String) {
        dotenvy::dotenv().ok();
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = connect(&url).await.unwrap();
        let sessions = PgSessionRepo::new(pool.clone());
        let code: ShortCode = RandomCodeGenerator::new(8).generate();
        sessions
            .create(code.as_str(), "", Utc::now())
            .await
            .unwrap();
        (PgNodeRepo::new(pool), sessions, code.as_str().to_string())
    }

    fn nv(key: &str, size: i64) -> NewVersion {
        NewVersion {
            size,
            content_type: "application/octet-stream".into(),
            sha256: None,
            blob_key: key.into(),
            thumb_key: None,
            client_created_at: None,
        }
    }

    #[tokio::test]
    async fn folders_are_unique_per_parent_and_listed_first() {
        let (repo, sessions, s) = setup().await;
        let now = Utc::now();
        let f = repo.create_folder(&s, None, "fixtures", now).await.unwrap();
        assert!(matches!(
            repo.create_folder(&s, None, "fixtures", now).await,
            Err(RepoError::NameTaken)
        ));
        // Same name is fine in another folder.
        repo.create_folder(&s, Some(f.id), "fixtures", now)
            .await
            .unwrap();
        repo.add_version(&s, None, "a.txt", nv("k1", 1), now)
            .await
            .unwrap();
        let list = repo.list(&s, None).await.unwrap();
        assert_eq!(
            list.iter()
                .map(|e| e.node.name.as_str())
                .collect::<Vec<_>>(),
            ["fixtures", "a.txt"]
        );
        assert_eq!(list[0].child_count, 1);
        // A file cannot be a parent.
        let (file, _) = repo
            .add_version(&s, None, "b.txt", nv("k2", 1), now)
            .await
            .unwrap();
        assert!(matches!(
            repo.create_folder(&s, Some(file.id), "x", now).await,
            Err(RepoError::Invalid(_))
        ));
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn same_name_adds_versions_and_latest_moves() {
        let (repo, sessions, s) = setup().await;
        let now = Utc::now();
        let (n1, v1) = repo
            .add_version(&s, None, "build.apk", nv("k-a", 10), now)
            .await
            .unwrap();
        let (n2, v2) = repo
            .add_version(&s, None, "build.apk", nv("k-b", 20), now)
            .await
            .unwrap();
        assert_eq!(n1.id, n2.id);
        assert_eq!((v1.version, v2.version), (1, 2));
        let versions = repo.versions(&s, n1.id).await.unwrap();
        assert_eq!(
            versions.iter().map(|v| v.version).collect::<Vec<_>>(),
            [2, 1]
        );
        assert!(versions[0].is_latest && versions[0].tags.contains(&"latest".to_string()));
        assert!(!versions[1].is_latest && !versions[1].tags.contains(&"latest".to_string()));
        let list = repo.list(&s, None).await.unwrap();
        assert_eq!(list[0].version_count, 2);
        assert_eq!(list[0].latest.as_ref().unwrap().size, 20);
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn versions_of_many_files_come_back_in_one_call_newest_first() {
        let (repo, sessions, s) = setup().await;
        let now = Utc::now();
        let (a, _) = repo
            .add_version(&s, None, "a", nv("k-a1", 1), now)
            .await
            .unwrap();
        repo.add_version(&s, None, "a", nv("k-a2", 2), now)
            .await
            .unwrap();
        let (b, _) = repo
            .add_version(&s, None, "b", nv("k-b1", 3), now)
            .await
            .unwrap();
        let (_, other_sessions, other) = setup().await;
        let (foreign, _) = repo
            .add_version(&other, None, "a", nv("k-o1", 4), now)
            .await
            .unwrap();

        let got = repo
            .versions_of(&s, &[a.id, b.id, foreign.id])
            .await
            .unwrap();
        assert_eq!(
            got[&a.id].iter().map(|v| v.version).collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(got[&b.id].len(), 1);
        // A node of another session is not visible, and an empty request asks nothing.
        assert!(!got.contains_key(&foreign.id));
        assert!(repo.versions_of(&s, &[]).await.unwrap().is_empty());
        other_sessions.delete(&other).await.unwrap();
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn tags_move_between_versions_and_can_be_found() {
        let (repo, sessions, s) = setup().await;
        let now = Utc::now();
        let (_, v1) = repo
            .add_version(&s, None, "build.apk", nv("k-a", 10), now)
            .await
            .unwrap();
        let (_, v2) = repo
            .add_version(&s, None, "build.apk", nv("k-b", 20), now)
            .await
            .unwrap();
        repo.add_tag(&s, v1.id, "stable").await.unwrap();
        let moved = repo.add_tag(&s, v2.id, "stable").await.unwrap();
        assert!(moved.tags.contains(&"stable".to_string()));
        let (_, found) = repo
            .find_version(&s, None, "build.apk", Some("stable"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, v2.id);
        let (_, latest) = repo
            .find_version(&s, None, "build.apk", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, v2.id);
        let (_, latest2) = repo
            .find_version(&s, None, "build.apk", Some("latest"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest2.id, v2.id);
        assert!(repo
            .find_version(&s, None, "build.apk", Some("nope"))
            .await
            .unwrap()
            .is_none());
        assert!(matches!(
            repo.add_tag(&s, v1.id, "latest").await,
            Err(RepoError::Invalid(_))
        ));
        repo.remove_tag(&s, v2.id, "stable").await.unwrap();
        assert!(repo
            .find_version(&s, None, "build.apk", Some("stable"))
            .await
            .unwrap()
            .is_none());
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_first_uploads_of_one_name_all_become_versions() {
        let (repo, sessions, s) = setup().await;
        let mut handles = Vec::new();
        for i in 0..8 {
            let (repo, s) = (repo.clone(), s.clone());
            handles.push(tokio::spawn(async move {
                repo.add_version(&s, None, "race.bin", nv(&format!("k{i}"), i), Utc::now())
                    .await
            }));
        }
        let mut versions = Vec::new();
        for h in handles {
            versions.push(h.await.unwrap().expect("no upload may fail").1.version);
        }
        versions.sort();
        assert_eq!(versions, (1..=8).collect::<Vec<_>>());
        let list = repo.list(&s, None).await.unwrap();
        assert_eq!(list.len(), 1, "one file, not one per upload");
        assert_eq!(list[0].version_count, 8);
        let all = repo.versions(&s, list[0].node.id).await.unwrap();
        assert_eq!(all.iter().filter(|v| v.is_latest).count(), 1);
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn the_file_limit_holds_even_when_uploads_race() {
        let (repo, sessions, s) = setup().await;
        let repo = repo.with_max_files(3);
        let now = Utc::now();
        repo.add_version(&s, None, "a", nv("k-a", 1), now)
            .await
            .unwrap();
        // A new version of an existing file is not a new file.
        repo.add_version(&s, None, "a", nv("k-a2", 1), now)
            .await
            .unwrap();

        // Six new names at once, two slots left: exactly two fit.
        let mut handles = Vec::new();
        for i in 0..6 {
            let (repo, s) = (repo.clone(), s.clone());
            handles.push(tokio::spawn(async move {
                repo.add_version(
                    &s,
                    None,
                    &format!("p{i}"),
                    nv(&format!("k{i}"), 1),
                    Utc::now(),
                )
                .await
            }));
        }
        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }
        let stored = results.iter().filter(|r| r.is_ok()).count();
        let refused = results
            .iter()
            .filter(|r| matches!(r, Err(RepoError::LimitReached(m)) if m.contains("maximum of 3 files")))
            .count();
        assert_eq!((stored, refused), (2, 4));
        assert_eq!(repo.count_files(&s).await.unwrap(), 3);

        // Versions still fit when the session is full, and a deleted file frees a slot.
        repo.add_version(&s, None, "a", nv("k-a3", 1), now)
            .await
            .unwrap();
        let first = repo.list(&s, None).await.unwrap()[0].node.id;
        repo.delete(&s, first).await.unwrap();
        repo.add_version(&s, None, "again", nv("k-new", 1), now)
            .await
            .unwrap();
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn versions_per_file_and_bytes_per_session_are_limited_even_in_parallel() {
        let (repo, sessions, s) = setup().await;
        let repo = repo.with_limits(Limits {
            max_files: 100,
            max_versions_per_file: 3,
            max_session_bytes: 100,
        });
        let now = Utc::now();
        for i in 0..3 {
            repo.add_version(&s, None, "a", nv(&format!("a{i}"), 10), now)
                .await
                .unwrap();
        }
        // A fourth version is refused, another file is not affected.
        let err = repo
            .add_version(&s, None, "a", nv("a3", 10), now)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, RepoError::LimitReached(m) if m.contains("maximum of 3 versions")),
            "{err}"
        );
        repo.add_version(&s, None, "b", nv("b0", 60), now)
            .await
            .unwrap();

        // 90 bytes are used. 20 more would pass 100, 10 more fit exactly.
        let err = repo
            .add_version(&s, None, "c", nv("c0", 20), now)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, RepoError::LimitReached(m) if m.contains("storage limit")),
            "{err}"
        );
        repo.add_version(&s, None, "c", nv("c0", 10), now)
            .await
            .unwrap();
        assert!(
            repo.add_version(&s, None, "c", nv("c1", 1), now)
                .await
                .is_err(),
            "100 of 100 bytes used"
        );

        // Parallel versions of "b" (1 version, limit 3, bytes already full): all refused, none lost.
        let (repo2, s2) = (repo.clone(), s.clone());
        let results = tokio::spawn(async move {
            let mut out = Vec::new();
            for i in 0..4 {
                out.push(
                    repo2
                        .add_version(&s2, None, "b", nv(&format!("b{}", i + 1), 0), Utc::now())
                        .await,
                );
            }
            out
        })
        .await
        .unwrap();
        // Zero-byte versions fit the byte limit, so exactly two more fit the version limit.
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
        let mut handles = Vec::new();
        for i in 0..6 {
            let (repo, s) = (repo.clone(), s.clone());
            handles.push(tokio::spawn(async move {
                repo.add_version(&s, None, "d", nv(&format!("d{i}"), 0), Utc::now())
                    .await
            }));
        }
        let mut stored = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                stored += 1;
            }
        }
        assert_eq!(
            stored, 3,
            "six parallel uploads of a new name, three versions fit"
        );
        sessions.delete(&s).await.unwrap();
    }

    #[tokio::test]
    async fn update_delete_and_keys() {
        let (repo, sessions, s) = setup().await;
        let now = Utc::now();
        let f = repo.create_folder(&s, None, "dir", now).await.unwrap();
        repo.add_version(
            &s,
            Some(f.id),
            "in.txt",
            NewVersion {
                thumb_key: Some("t1".into()),
                ..nv("k1", 1)
            },
            now,
        )
        .await
        .unwrap();
        let (other, _) = repo
            .add_version(&s, None, "top.txt", nv("k2", 1), now)
            .await
            .unwrap();
        let patched = repo
            .update(
                &s,
                other.id,
                NodePatch {
                    name: Some("renamed.txt".into()),
                    note: Some("hello".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            (patched.name.as_str(), patched.note.as_str()),
            ("renamed.txt", "hello")
        );
        assert!(matches!(
            repo.update(
                &s,
                other.id,
                NodePatch {
                    name: Some("dir".into()),
                    note: None
                }
            )
            .await,
            Err(RepoError::NameTaken)
        ));
        let mut all = repo.all_keys(&s).await.unwrap();
        all.sort();
        assert_eq!(all, ["k1", "k2", "t1"]);
        let mut removed = repo.delete(&s, f.id).await.unwrap();
        removed.sort();
        assert_eq!(removed, ["k1", "t1"]);
        assert!(matches!(
            repo.delete(&s, f.id).await,
            Err(RepoError::NotFound)
        ));
        assert!(repo.get("other-session", other.id).await.unwrap().is_none());
        sessions.delete(&s).await.unwrap();
        assert!(
            repo.get(&s, other.id).await.unwrap().is_none(),
            "session delete cascades"
        );
    }
}
