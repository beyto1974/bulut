//! JSON logging to stdout. The level comes from `LOG_LEVEL`.

use tracing_subscriber::EnvFilter;

/// Turns the configured level into a filter. Unknown levels fall back to `info`.
pub fn filter_for(level: &str) -> EnvFilter {
    let level = match level.to_ascii_lowercase().as_str() {
        l @ ("error" | "warn" | "info" | "debug" | "trace") => l.to_string(),
        _ => "info".to_string(),
    };
    EnvFilter::new(level)
}

pub fn init(level: &str) {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_env_filter(filter_for(level))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_levels_are_kept() {
        assert_eq!(filter_for("debug").to_string(), "debug");
        assert_eq!(filter_for("WARN").to_string(), "warn");
    }

    #[test]
    fn unknown_level_falls_back_to_info() {
        assert_eq!(filter_for("loud").to_string(), "info");
        assert_eq!(filter_for("").to_string(), "info");
    }
}
