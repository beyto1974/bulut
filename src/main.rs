use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use bulut::adapters::{postgres, s3_blobs::S3BlobStore};
use bulut::storage_config::StorageConfig;
use bulut::{app, config::Config, http, logging, VERSION};

/// `bulut healthcheck` is used by the container HEALTHCHECK (the image has no curl).
fn healthcheck(port: u16) -> i32 {
    let run = || -> std::io::Result<bool> {
        let mut s = TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(2)))?;
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut buf = String::new();
        s.read_to_string(&mut buf)?;
        Ok(buf.starts_with("HTTP/1.1 200"))
    };
    if run().unwrap_or(false) {
        0
    } else {
        1
    }
}

/// Resolves on SIGINT or SIGTERM. `docker stop` and redeploys send SIGTERM, and the process is PID 1.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}

fn exit_with(message: String) -> ! {
    eprintln!("{message}");
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    let config =
        Config::from_env().unwrap_or_else(|e| exit_with(format!("configuration error: {e}")));
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(healthcheck(config.port));
    }
    logging::init(&config.log_level);
    let storage = StorageConfig::from_env()
        .unwrap_or_else(|e| exit_with(format!("configuration error: {e}")));

    let pool = postgres::connect(&storage.database_url)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "cannot connect to the database or run migrations");
            std::process::exit(1);
        });
    let blobs = Arc::new(S3BlobStore::new(&storage));
    if let Err(e) = blobs.ensure_bucket().await {
        tracing::error!(error = %e, bucket = %storage.s3_bucket, "cannot reach the storage bucket");
        std::process::exit(1);
    }

    let sweep_every = Duration::from_secs(config.sweep_interval_secs.max(60));
    let addr = std::net::SocketAddr::new(config.bind_addr, config.port).to_string();
    let env = config.app_env.as_str();
    let idle_days = config.session_idle_ttl_days;
    let state = app::build_state(config, pool, blobs, VERSION);
    app::spawn_sweeper(
        state.sessions.clone(),
        state.uploads.clone(),
        state.orphans.clone(),
        sweep_every,
    );

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(%addr, error = %e, "cannot bind");
            std::process::exit(1);
        });
    tracing::info!(%addr, env, version = VERSION, idle_days, "bulut started");

    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}
