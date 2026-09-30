pub mod adapters;
pub mod app;
pub mod config;
pub mod domain;
pub mod http;
pub mod llms;
pub mod logging;
pub mod ports;
pub mod services;
pub mod storage_config;

/// Version baked in at build time from the `VERSION` file (bumped by CI).
pub const VERSION: &str = env!("BULUT_VERSION");
