pub mod adapters;
pub mod config;
pub mod domain;
pub mod http;
pub mod logging;
pub mod ports;

/// Version baked in at build time from the `VERSION` file (bumped by CI).
pub const VERSION: &str = env!("BULUT_VERSION");
