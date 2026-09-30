//! Wiring: builds the services from their adapters.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::adapters::postgres::PgSessionRepo;
use crate::adapters::postgres_nodes::{Limits, PgNodeRepo};
use crate::adapters::postgres_uploads::PgUploadRepo;
use crate::config::Config;
use crate::http::AppState;
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::SystemClock;
use crate::ports::code_generator::RandomCodeGenerator;
use crate::services::session_service::SessionService;
use crate::services::tree_service::TreeService;
use crate::services::upload_service::UploadService;

pub fn build_state(
    config: Config,
    pool: PgPool,
    blobs: Arc<dyn BlobStore>,
    version: &'static str,
) -> AppState {
    let nodes = Arc::new(PgNodeRepo::new(pool.clone()).with_limits(Limits {
        max_files: i64::from(config.max_files_per_session),
        max_versions_per_file: i64::from(config.max_versions_per_file),
        max_session_bytes: i64::try_from(config.max_session_bytes).unwrap_or(i64::MAX),
    }));
    let clock = Arc::new(SystemClock);
    let uploads = Arc::new(UploadService {
        folders_enabled: config.folders_enabled,
        uploads: Arc::new(PgUploadRepo::new(pool.clone())),
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        clock: clock.clone(),
        part_size: config.chunk_size,
        max_file_bytes: config.max_file_bytes,
        max_files: config.max_files_per_session,
        max_pending: config.max_pending_uploads,
        max_tags: config.max_tags_per_version,
    });
    let sessions = Arc::new(SessionService {
        sessions: Arc::new(PgSessionRepo::new(pool)),
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        uploads: uploads.clone(),
        codes: Arc::new(RandomCodeGenerator::new(config.code_length)),
        clock: clock.clone(),
        code_length: config.code_length,
        idle_ttl_days: config.session_idle_ttl_days,
    });
    let tree = Arc::new(TreeService {
        folders_enabled: config.folders_enabled,
        max_tags: config.max_tags_per_version,
        nodes,
        blobs,
        clock,
    });
    AppState {
        config: Arc::new(config),
        version,
        sessions,
        tree,
        uploads,
    }
}

/// Periodically deletes sessions that have been idle longer than the configured window.
pub fn spawn_sweeper(sessions: Arc<SessionService>, uploads: Arc<UploadService>, every: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
            // Uploads nobody finished within a day only hold space in the store.
            match uploads.cleanup_stale(chrono::Duration::hours(24)).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, "aborted abandoned uploads"),
                Err(e) => tracing::error!(error = %e, "abandoned upload cleanup failed"),
            }
            match sessions.sweep_idle().await {
                Ok(purged) if !purged.is_empty() => {
                    tracing::info!(count = purged.len(), "idle sweep finished")
                }
                Ok(_) => tracing::debug!("idle sweep found nothing"),
                Err(e) => tracing::error!(error = %e, "idle sweep failed"),
            }
        }
    });
}
