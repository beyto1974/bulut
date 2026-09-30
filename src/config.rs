//! Runtime configuration, read from environment variables (loaded from `.env` by compose).

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Production,
    Testing,
    Development,
}

impl AppEnv {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "production" | "prod" => Ok(Self::Production),
            "testing" | "test" => Ok(Self::Testing),
            "development" | "dev" => Ok(Self::Development),
            other => Err(ConfigError::Invalid("APP_ENV", other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Testing => "testing",
            Self::Development => "development",
        }
    }

    pub fn is_production(self) -> bool {
        self == Self::Production
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Invalid(&'static str, String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(key, value) => write!(f, "invalid value for {key}: {value:?}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub app_env: AppEnv,
    pub log_level: String,
    pub port: u16,
    pub base_url: String,
    pub code_length: usize,
    pub chunk_size: usize,
    pub session_idle_ttl_days: u32,
    pub sweep_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let vars: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&vars)
    }

    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let get = |key: &str| vars.get(key).map(|v| v.trim()).filter(|v| !v.is_empty());
        fn num<T: std::str::FromStr>(
            key: &'static str,
            raw: Option<&str>,
            default: T,
        ) -> Result<T, ConfigError> {
            match raw {
                None => Ok(default),
                Some(v) => v
                    .parse()
                    .map_err(|_| ConfigError::Invalid(key, v.to_string())),
            }
        }

        let app_env = match get("APP_ENV") {
            Some(v) => AppEnv::parse(v)?,
            None => AppEnv::Production,
        };
        let port: u16 = num("PORT", get("PORT"), 8080)?;
        let code_length: usize = num("CODE_LENGTH", get("CODE_LENGTH"), 5)?;
        if !(3..=12).contains(&code_length) {
            return Err(ConfigError::Invalid("CODE_LENGTH", code_length.to_string()));
        }
        let chunk_size: usize = num("CHUNK_SIZE", get("CHUNK_SIZE"), 8 * 1024 * 1024)?;
        if chunk_size < 5 * 1024 * 1024 {
            // S3 multipart parts (except the last) must be at least 5 MiB.
            return Err(ConfigError::Invalid("CHUNK_SIZE", chunk_size.to_string()));
        }
        let session_idle_ttl_days: u32 =
            num("SESSION_IDLE_TTL_DAYS", get("SESSION_IDLE_TTL_DAYS"), 7)?;
        if session_idle_ttl_days == 0 {
            return Err(ConfigError::Invalid("SESSION_IDLE_TTL_DAYS", "0".into()));
        }
        Ok(Self {
            app_env,
            log_level: get("LOG_LEVEL").unwrap_or("info").to_ascii_lowercase(),
            port,
            base_url: get("BASE_URL")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| format!("http://localhost:{port}")),
            code_length,
            chunk_size,
            session_idle_ttl_days,
            sweep_interval_secs: num("SWEEP_INTERVAL_SECS", get("SWEEP_INTERVAL_SECS"), 3600)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_are_production_and_safe() {
        let c = Config::from_map(&vars(&[])).unwrap();
        assert_eq!(c.app_env, AppEnv::Production);
        assert_eq!(c.log_level, "info");
        assert_eq!(c.port, 8080);
        assert_eq!(c.code_length, 5);
        assert_eq!(c.session_idle_ttl_days, 7);
        assert_eq!(c.base_url, "http://localhost:8080");
    }

    #[test]
    fn reads_overrides_and_trims_base_url() {
        let c = Config::from_map(&vars(&[
            ("APP_ENV", "testing"),
            ("LOG_LEVEL", "DEBUG"),
            ("PORT", "9000"),
            ("BASE_URL", "https://bulut.dev/"),
            ("SESSION_IDLE_TTL_DAYS", "3"),
        ]))
        .unwrap();
        assert_eq!(c.app_env, AppEnv::Testing);
        assert!(!c.app_env.is_production());
        assert_eq!(c.log_level, "debug");
        assert_eq!(c.port, 9000);
        assert_eq!(c.base_url, "https://bulut.dev");
        assert_eq!(c.session_idle_ttl_days, 3);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::from_map(&vars(&[("APP_ENV", "staging")])).is_err());
        assert!(Config::from_map(&vars(&[("PORT", "abc")])).is_err());
        assert!(Config::from_map(&vars(&[("CODE_LENGTH", "1")])).is_err());
        assert!(Config::from_map(&vars(&[("CHUNK_SIZE", "1024")])).is_err());
        assert!(Config::from_map(&vars(&[("SESSION_IDLE_TTL_DAYS", "0")])).is_err());
    }
}
