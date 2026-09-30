//! Wiring: builds the services from their adapters.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::adapters::postgres::PgSessionRepo;
use crate::adapters::postgres_nodes::PgNodeRepo;
use crate::config::Config;
use crate::http::AppState;
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::SystemClock;
use crate::ports::code_generator::RandomCodeGenerator;
use crate::services::session_service::SessionService;
use crate::services::tree_service::TreeService;

pub fn build_state(
    config: Config,
    pool: PgPool,
    blobs: Arc<dyn BlobStore>,
    version: &'static str,
) -> AppState {
    let nodes = Arc::new(PgNodeRepo::new(pool.clone()));
    let clock = Arc::new(SystemClock);
    let sessions = Arc::new(SessionService {
        sessions: Arc::new(PgSessionRepo::new(pool)),
        nodes: nodes.clone(),
        blobs: blobs.clone(),
        codes: Arc::new(RandomCodeGenerator::new(config.code_length)),
        clock: clock.clone(),
        code_length: config.code_length,
        idle_ttl_days: config.session_idle_ttl_days,
    });
    let tree = Arc::new(TreeService {
        nodes,
        blobs,
        clock,
    });
    AppState {
        config: Arc::new(config),
        version,
        sessions,
        tree,
    }
}

/// Periodically deletes sessions that have been idle longer than the configured window.
pub fn spawn_sweeper(sessions: Arc<SessionService>, every: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
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
