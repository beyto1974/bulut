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
    Missing(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(key, value) => write!(f, "invalid value for {key}: {value:?}"),
            Self::Missing(key) => write!(f, "{key} is required"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub app_env: AppEnv,
    pub log_level: String,
    pub port: u16,
    /// Address to listen on. `0.0.0.0` (all interfaces) suits a container, `127.0.0.1` a bare install.
    pub bind_addr: std::net::IpAddr,
    pub base_url: String,
    pub code_length: usize,
    pub chunk_size: usize,
    pub max_file_bytes: u64,
    /// Most files one session can hold. New versions of an existing file do not count.
    pub max_files_per_session: u32,
    /// Most versions one file keeps.
    pub max_versions_per_file: u32,
    /// Most bytes of stored versions in one session.
    pub max_session_bytes: u64,
    /// Most unfinished chunked uploads in one session.
    pub max_pending_uploads: u32,
    /// Most tags on one version.
    pub max_tags_per_version: u32,
    /// Folder support is built but switched off until the UI handles it.
    pub folders_enabled: bool,
    pub session_idle_ttl_days: u32,
    pub sweep_interval_secs: u64,
    /// Objects in the bucket that no file refers to are deleted once they are this many hours
    /// old. `0` switches the clean-up off.
    pub orphan_grace_hours: u32,
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
        let bind_addr: std::net::IpAddr = num(
            "BIND_ADDR",
            get("BIND_ADDR"),
            std::net::Ipv4Addr::UNSPECIFIED.into(),
        )?;
        let code_length: usize = num("CODE_LENGTH", get("CODE_LENGTH"), 5)?;
        if !(3..=12).contains(&code_length) {
            return Err(ConfigError::Invalid("CODE_LENGTH", code_length.to_string()));
        }
        let chunk_size: usize = num("CHUNK_SIZE", get("CHUNK_SIZE"), 8 * 1024 * 1024)?;
        if chunk_size < 5 * 1024 * 1024 {
            // S3 multipart parts (except the last) must be at least 5 MiB.
            return Err(ConfigError::Invalid("CHUNK_SIZE", chunk_size.to_string()));
        }
        let max_file_bytes: u64 = num("MAX_FILE_BYTES", get("MAX_FILE_BYTES"), 1024 * 1024 * 1024)?;
        if max_file_bytes == 0 {
            return Err(ConfigError::Invalid("MAX_FILE_BYTES", "0".into()));
        }
        let max_files_per_session: u32 =
            num("MAX_FILES_PER_SESSION", get("MAX_FILES_PER_SESSION"), 100)?;
        if !(1..=100_000).contains(&max_files_per_session) {
            return Err(ConfigError::Invalid(
                "MAX_FILES_PER_SESSION",
                max_files_per_session.to_string(),
            ));
        }
        let max_versions_per_file: u32 =
            num("MAX_VERSIONS_PER_FILE", get("MAX_VERSIONS_PER_FILE"), 50)?;
        let max_session_bytes: u64 = num(
            "MAX_SESSION_BYTES",
            get("MAX_SESSION_BYTES"),
            10 * 1024 * 1024 * 1024,
        )?;
        let max_pending_uploads: u32 = num("MAX_PENDING_UPLOADS", get("MAX_PENDING_UPLOADS"), 10)?;
        let max_tags_per_version: u32 =
            num("MAX_TAGS_PER_VERSION", get("MAX_TAGS_PER_VERSION"), 20)?;
        for (key, value) in [
            ("MAX_VERSIONS_PER_FILE", u64::from(max_versions_per_file)),
            ("MAX_SESSION_BYTES", max_session_bytes),
            ("MAX_PENDING_UPLOADS", u64::from(max_pending_uploads)),
            ("MAX_TAGS_PER_VERSION", u64::from(max_tags_per_version)),
        ] {
            if value == 0 || value > i64::MAX as u64 {
                return Err(ConfigError::Invalid(key, value.to_string()));
            }
        }
        let folders_enabled = match get("FOLDERS_ENABLED") {
            None => false,
            Some(v) => match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                other => return Err(ConfigError::Invalid("FOLDERS_ENABLED", other.to_string())),
            },
        };
        if max_file_bytes.div_ceil(chunk_size as u64) > 10_000 {
            // S3 multipart uploads have at most 10000 parts. Fail at startup, not after gigabytes.
            return Err(ConfigError::Invalid(
                "MAX_FILE_BYTES",
                format!("{max_file_bytes} needs more than 10000 parts of CHUNK_SIZE {chunk_size}"),
            ));
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
            bind_addr,
            base_url: get("BASE_URL")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| format!("http://localhost:{port}")),
            code_length,
            chunk_size,
            max_file_bytes,
            max_files_per_session,
            max_versions_per_file,
            max_session_bytes,
            max_pending_uploads,
            max_tags_per_version,
            folders_enabled,
            session_idle_ttl_days,
            sweep_interval_secs: num("SWEEP_INTERVAL_SECS", get("SWEEP_INTERVAL_SECS"), 3600)?,
            orphan_grace_hours: num("ORPHAN_GRACE_HOURS", get("ORPHAN_GRACE_HOURS"), 24)?,
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
        assert_eq!(c.bind_addr.to_string(), "0.0.0.0");
        assert_eq!(c.code_length, 5);
        assert_eq!(c.session_idle_ttl_days, 7);
        assert_eq!(c.orphan_grace_hours, 24);
        assert_eq!(c.max_file_bytes, 1024 * 1024 * 1024);
        assert!(!c.folders_enabled, "folders are off by default");
        assert_eq!(c.max_files_per_session, 100);
        assert_eq!(
            (
                c.max_versions_per_file,
                c.max_pending_uploads,
                c.max_tags_per_version
            ),
            (50, 10, 20)
        );
        assert_eq!(c.max_session_bytes, 10 * 1024 * 1024 * 1024);
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
            ("ORPHAN_GRACE_HOURS", "0"),
        ]))
        .unwrap();
        assert_eq!(c.app_env, AppEnv::Testing);
        assert!(!c.app_env.is_production());
        assert_eq!(c.log_level, "debug");
        assert_eq!(c.port, 9000);
        assert_eq!(c.base_url, "https://bulut.dev");
        assert_eq!(c.session_idle_ttl_days, 3);
        assert_eq!(c.orphan_grace_hours, 0, "0 switches the clean-up off");
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::from_map(&vars(&[("APP_ENV", "staging")])).is_err());
        assert!(Config::from_map(&vars(&[("PORT", "abc")])).is_err());
        assert!(Config::from_map(&vars(&[("BIND_ADDR", "everywhere")])).is_err());
        assert_eq!(
            Config::from_map(&vars(&[("BIND_ADDR", "127.0.0.1")]))
                .unwrap()
                .bind_addr
                .to_string(),
            "127.0.0.1"
        );
        assert!(Config::from_map(&vars(&[("CODE_LENGTH", "1")])).is_err());
        assert!(Config::from_map(&vars(&[("CHUNK_SIZE", "1024")])).is_err());
        assert!(Config::from_map(&vars(&[("SESSION_IDLE_TTL_DAYS", "0")])).is_err());
        assert!(Config::from_map(&vars(&[("MAX_FILE_BYTES", "0")])).is_err());
        assert!(Config::from_map(&vars(&[("FOLDERS_ENABLED", "maybe")])).is_err());
        assert!(Config::from_map(&vars(&[("ORPHAN_GRACE_HOURS", "-1")])).is_err());
        assert!(Config::from_map(&vars(&[("MAX_FILES_PER_SESSION", "0")])).is_err());
        for key in [
            "MAX_VERSIONS_PER_FILE",
            "MAX_SESSION_BYTES",
            "MAX_PENDING_UPLOADS",
            "MAX_TAGS_PER_VERSION",
        ] {
            assert!(Config::from_map(&vars(&[(key, "0")])).is_err(), "{key}");
        }
        assert!(Config::from_map(&vars(&[("MAX_FILES_PER_SESSION", "many")])).is_err());
        // 100 GB in 8 MiB parts is 11920 parts, more than S3 allows.
        assert!(Config::from_map(&vars(&[("MAX_FILE_BYTES", "100000000000")])).is_err());
        assert!(Config::from_map(&vars(&[
            ("MAX_FILE_BYTES", "100000000000"),
            ("CHUNK_SIZE", "16777216")
        ]))
        .is_ok());
    }
}
