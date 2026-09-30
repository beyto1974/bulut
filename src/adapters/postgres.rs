//! Postgres implementation of the repositories.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

use crate::domain::session::Session;
use crate::ports::session_repo::{RepoError, SessionRepo};

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

#[derive(Clone)]
pub struct PgSessionRepo {
    pool: PgPool,
}

impl PgSessionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_session(row: &sqlx::postgres::PgRow) -> Session {
    Session {
        code: row.get("code"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        last_activity_at: row.get("last_activity_at"),
    }
}

#[async_trait]
impl SessionRepo for PgSessionRepo {
    async fn create(
        &self,
        code: &str,
        description: &str,
        now: DateTime<Utc>,
    ) -> Result<Session, RepoError> {
        let res = sqlx::query(
            "INSERT INTO sessions (code, description, created_at, last_activity_at)
             VALUES ($1, $2, $3, $3)
             RETURNING code, description, created_at, last_activity_at",
        )
        .bind(code)
        .bind(description)
        .bind(now)
        .fetch_one(&self.pool)
        .await;
        match res {
            Ok(row) => Ok(row_to_session(&row)),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(RepoError::CodeTaken),
            Err(e) => Err(storage(e)),
        }
    }

    async fn get(&self, code: &str) -> Result<Option<Session>, RepoError> {
        let row = sqlx::query(
            "SELECT code, description, created_at, last_activity_at
             FROM sessions WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.as_ref().map(row_to_session))
    }

    async fn set_description(&self, code: &str, description: &str) -> Result<(), RepoError> {
        let r = sqlx::query("UPDATE sessions SET description = $2 WHERE code = $1")
            .bind(code)
            .bind(description)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        if r.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }
        Ok(())
    }

    async fn touch(
        &self,
        code: &str,
        now: DateTime<Utc>,
        throttle_secs: i64,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query(
            "UPDATE sessions SET last_activity_at = $2
             WHERE code = $1 AND last_activity_at <= $3",
        )
        .bind(code)
        .bind(now)
        .bind(now - Duration::seconds(throttle_secs))
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(r.rows_affected() > 0)
    }

    async fn delete(&self, code: &str) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM sessions WHERE code = $1")
            .bind(code)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn idle_before(&self, cutoff: DateTime<Utc>) -> Result<Vec<String>, RepoError> {
        let rows = sqlx::query("SELECT code FROM sessions WHERE last_activity_at <= $1")
            .bind(cutoff)
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        Ok(rows.iter().map(|r| r.get("code")).collect())
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests. They need `DATABASE_URL` (the shared dev Postgres, see README) and
    //! use unique codes, cleaning up after themselves.
    use super::*;
    use crate::domain::short_code::ShortCode;
    use crate::ports::code_generator::{CodeGenerator, RandomCodeGenerator};

    async fn repo() -> PgSessionRepo {
        dotenvy::dotenv().ok();
        let url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        PgSessionRepo::new(connect(&url).await.expect("connect and migrate"))
    }

    fn fresh_code() -> ShortCode {
        RandomCodeGenerator::new(8).generate()
    }

    #[tokio::test]
    async fn create_get_and_duplicate_code() {
        let repo = repo().await;
        let code = fresh_code();
        let now = Utc::now();
        let s = repo.create(code.as_str(), "demo", now).await.unwrap();
        assert_eq!(s.description, "demo");
        assert_eq!(
            repo.get(code.as_str()).await.unwrap().unwrap().code,
            code.as_str()
        );
        assert!(matches!(
            repo.create(code.as_str(), "again", now).await,
            Err(RepoError::CodeTaken)
        ));
        repo.delete(code.as_str()).await.unwrap();
        assert!(repo.get(code.as_str()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn description_update() {
        let repo = repo().await;
        let code = fresh_code();
        repo.create(code.as_str(), "", Utc::now()).await.unwrap();
        repo.set_description(code.as_str(), "new text")
            .await
            .unwrap();
        assert_eq!(
            repo.get(code.as_str()).await.unwrap().unwrap().description,
            "new text"
        );
        assert!(matches!(
            repo.set_description("nope-nope", "x").await,
            Err(RepoError::NotFound)
        ));
        repo.delete(code.as_str()).await.unwrap();
    }

    #[tokio::test]
    async fn touch_is_throttled_and_moves_activity_forward() {
        let repo = repo().await;
        let code = fresh_code();
        let t0 = Utc::now() - Duration::hours(1);
        repo.create(code.as_str(), "", t0).await.unwrap();
        // Within the throttle window: no write.
        assert!(!repo
            .touch(code.as_str(), t0 + Duration::seconds(10), 60)
            .await
            .unwrap());
        // Past the window: written.
        let t1 = t0 + Duration::seconds(120);
        assert!(repo.touch(code.as_str(), t1, 60).await.unwrap());
        let got = repo.get(code.as_str()).await.unwrap().unwrap();
        assert!((got.last_activity_at - t1).num_milliseconds().abs() < 5);
        repo.delete(code.as_str()).await.unwrap();
    }

    #[tokio::test]
    async fn finds_only_idle_sessions_and_delete_cascades() {
        let repo = repo().await;
        let (old, recent) = (fresh_code(), fresh_code());
        let now = Utc::now();
        repo.create(old.as_str(), "", now - Duration::days(9))
            .await
            .unwrap();
        repo.create(recent.as_str(), "", now - Duration::days(1))
            .await
            .unwrap();
        let idle = repo.idle_before(now - Duration::days(7)).await.unwrap();
        assert!(idle.contains(&old.as_str().to_string()));
        assert!(!idle.contains(&recent.as_str().to_string()));
        repo.delete(old.as_str()).await.unwrap();
        repo.delete(recent.as_str()).await.unwrap();
    }
}
