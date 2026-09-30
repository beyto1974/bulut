//! Postgres implementation of the file tree repository.

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

#[derive(Clone)]
pub struct PgNodeRepo {
    pool: PgPool,
}

impl PgNodeRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
        let mut tx = self.pool.begin().await.map_err(storage)?;
        check_parent(&mut tx, session, parent).await?;

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
                node
            }
            None => {
                let row = sqlx::query(&format!(
                    "INSERT INTO nodes (id, session_code, parent_id, kind, name, created_at)
                     VALUES ($1, $2, $3, 'file', $4, $5) RETURNING {NODE_COLS}"
                ))
                .bind(Uuid::new_v4())
                .bind(session)
                .bind(parent)
                .bind(name)
                .bind(now)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| {
                    if is_unique(&e) {
                        RepoError::NameTaken
                    } else {
                        storage(e)
                    }
                })?;
                node_from(&row)
            }
        };

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
        Ok((node, version_from(&row)))
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
            .create(code.as_str(), "", None, Utc::now())
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
