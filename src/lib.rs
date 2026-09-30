pub mod config;
pub mod http;
pub mod logging;

/// Version baked in at build time from the `VERSION` file (bumped by CI).
pub const VERSION: &str = env!("BULUT_VERSION");
