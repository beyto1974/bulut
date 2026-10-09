use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::domain::upload_link::UploadLink;
use crate::ports::link_repo::LinkRepo;
use crate::ports::session_repo::RepoError;

#[derive(Clone)]
pub struct PgLinkRepo {
    pool: PgPool,
}

impl PgLinkRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

#[async_trait]
impl LinkRepo for PgLinkRepo {
    async fn create(&self, l: &UploadLink) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO upload_links (id, session_code, files_total, files_left, expires_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&l.id)
        .bind(&l.session_code)
        .bind(l.files_total)
        .bind(l.files_left)
        .bind(l.expires_at)
        .bind(l.created_at)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn consume(&self, id: &str, now: DateTime<Utc>) -> Result<Option<UploadLink>, RepoError> {
        let row = sqlx::query(
            "UPDATE upload_links SET files_left = files_left - 1
             WHERE id = $1 AND files_left > 0 AND expires_at > $2
             RETURNING id, session_code, files_total, files_left, expires_at, created_at",
        )
        .bind(id)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.map(|r| UploadLink {
            id: r.get("id"),
            session_code: r.get("session_code"),
            files_total: r.get("files_total"),
            files_left: r.get("files_left"),
            expires_at: r.get("expires_at"),
            created_at: r.get("created_at"),
        }))
    }

    async fn release(&self, id: &str) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE upload_links SET files_left = files_left + 1 WHERE id = $1 AND files_left < files_total",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn purge_expired(&self, cutoff: DateTime<Utc>) -> Result<u64, RepoError> {
        let r = sqlx::query("DELETE FROM upload_links WHERE expires_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(r.rows_affected())
    }
}
