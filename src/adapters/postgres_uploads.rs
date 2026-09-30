use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};
use uuid::Uuid;

use crate::domain::upload::Upload;
use crate::ports::session_repo::RepoError;
use crate::ports::upload_repo::UploadRepo;

#[derive(Clone)]
pub struct PgUploadRepo {
    pool: PgPool,
}

impl PgUploadRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const COLS: &str = "id, session_code, parent_id, name, content_type, size, part_size, blob_key,
    s3_upload_id, client_created_at, created_at";

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

fn upload_from(r: &PgRow) -> Upload {
    Upload {
        id: r.get("id"),
        session_code: r.get("session_code"),
        parent_id: r.get("parent_id"),
        name: r.get("name"),
        content_type: r.get("content_type"),
        size: r.get("size"),
        part_size: r.get("part_size"),
        blob_key: r.get("blob_key"),
        s3_upload_id: r.get("s3_upload_id"),
        client_created_at: r.get("client_created_at"),
        created_at: r.get("created_at"),
    }
}

#[async_trait]
impl UploadRepo for PgUploadRepo {
    async fn create(&self, u: &Upload) -> Result<(), RepoError> {
        sqlx::query(&format!(
            "INSERT INTO uploads ({COLS}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        ))
        .bind(u.id)
        .bind(&u.session_code)
        .bind(u.parent_id)
        .bind(&u.name)
        .bind(&u.content_type)
        .bind(u.size)
        .bind(u.part_size)
        .bind(&u.blob_key)
        .bind(&u.s3_upload_id)
        .bind(u.client_created_at)
        .bind(u.created_at)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn get(&self, session: &str, id: Uuid) -> Result<Option<Upload>, RepoError> {
        let row = sqlx::query(&format!(
            "SELECT {COLS} FROM uploads WHERE id = $1 AND session_code = $2"
        ))
        .bind(id)
        .bind(session)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.as_ref().map(upload_from))
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM uploads WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn for_session(&self, session: &str) -> Result<Vec<Upload>, RepoError> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM uploads WHERE session_code = $1"
        ))
        .bind(session)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(rows.iter().map(upload_from).collect())
    }

    async fn started_before(&self, cutoff: DateTime<Utc>) -> Result<Vec<Upload>, RepoError> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM uploads WHERE created_at <= $1"
        ))
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(rows.iter().map(upload_from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::postgres::{connect, PgSessionRepo};
    use crate::ports::session_repo::SessionRepo;
    use chrono::Duration;

    fn sample(session: &str, created_at: DateTime<Utc>) -> Upload {
        Upload {
            id: Uuid::new_v4(),
            session_code: session.into(),
            parent_id: None,
            name: "big.bin".into(),
            content_type: "application/octet-stream".into(),
            size: 123,
            part_size: 10,
            blob_key: format!("{session}/x"),
            s3_upload_id: "up-1".into(),
            client_created_at: None,
            created_at,
        }
    }

    #[tokio::test]
    async fn roundtrip_lookup_scoping_and_cascade() {
        dotenvy::dotenv().ok();
        let pool = connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let sessions = PgSessionRepo::new(pool.clone());
        let repo = PgUploadRepo::new(pool);
        let code = format!("up{}", &Uuid::new_v4().simple().to_string()[..6]);
        // Postgres keeps microseconds, so compare at that precision.
        let now = DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).unwrap();
        sessions.create(&code, "", now).await.unwrap();

        let old = sample(&code, now - Duration::days(2));
        let fresh = sample(&code, now);
        repo.create(&old).await.unwrap();
        repo.create(&fresh).await.unwrap();

        assert_eq!(
            repo.get(&code, fresh.id).await.unwrap(),
            Some(fresh.clone())
        );
        assert_eq!(repo.get("someone-else", fresh.id).await.unwrap(), None);
        assert_eq!(repo.for_session(&code).await.unwrap().len(), 2);
        let stale = repo.started_before(now - Duration::days(1)).await.unwrap();
        assert!(stale.iter().any(|u| u.id == old.id));
        assert!(!stale.iter().any(|u| u.id == fresh.id));

        repo.delete(old.id).await.unwrap();
        assert_eq!(repo.get(&code, old.id).await.unwrap(), None);
        sessions.delete(&code).await.unwrap();
        assert_eq!(repo.get(&code, fresh.id).await.unwrap(), None, "cascade");
    }
}
