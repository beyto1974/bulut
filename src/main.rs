use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use bulut::{config::Config, http, logging, VERSION};

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

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error: {e}");
            std::process::exit(2);
        }
    };
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(healthcheck(config.port));
    }
    logging::init(&config.log_level);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(%addr, error = %e, "cannot bind");
            std::process::exit(1);
        });
    tracing::info!(%addr, env = config.app_env.as_str(), version = VERSION, "bulut started");

    let app = http::router(http::AppState {
        config,
        version: VERSION,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
