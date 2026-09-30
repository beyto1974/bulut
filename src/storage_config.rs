//! Database and object storage settings. Kept apart from `Config` so secrets never end up in
//! something that gets logged, and so tests of plain settings need no database.

use std::collections::HashMap;

use crate::config::ConfigError;

#[derive(Clone)]
pub struct StorageConfig {
    pub database_url: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: String,
    pub s3_access_key_id: String,
    pub s3_secret_access_key: String,
    pub s3_region: String,
}

impl std::fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageConfig")
            .field("s3_endpoint", &self.s3_endpoint)
            .field("s3_bucket", &self.s3_bucket)
            .field("s3_region", &self.s3_region)
            .finish_non_exhaustive()
    }
}

impl StorageConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let vars: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&vars)
    }

    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let get = |key: &str| {
            vars.get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let need = |key: &'static str| get(key).ok_or(ConfigError::Missing(key));
        Ok(Self {
            database_url: need("DATABASE_URL")?,
            s3_endpoint: get("S3_ENDPOINT_URL"),
            s3_bucket: need("S3_BUCKET")?,
            s3_access_key_id: need("S3_ACCESS_KEY_ID")?,
            s3_secret_access_key: need("S3_SECRET_ACCESS_KEY")?,
            s3_region: get("S3_REGION").unwrap_or_else(|| "us-east-1".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> HashMap<String, String> {
        [
            ("DATABASE_URL", "postgresql://u:secret-pw@db/x"),
            ("S3_BUCKET", "b"),
            ("S3_ACCESS_KEY_ID", "id"),
            ("S3_SECRET_ACCESS_KEY", "secret-key"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn reads_required_values_with_defaults() {
        let c = StorageConfig::from_map(&full()).unwrap();
        assert_eq!(c.s3_region, "us-east-1");
        assert_eq!(c.s3_endpoint, None);
    }

    #[test]
    fn names_the_missing_setting() {
        let mut v = full();
        v.remove("S3_BUCKET");
        assert_eq!(
            StorageConfig::from_map(&v).unwrap_err(),
            ConfigError::Missing("S3_BUCKET")
        );
    }

    #[test]
    fn debug_output_hides_secrets() {
        let shown = format!("{:?}", StorageConfig::from_map(&full()).unwrap());
        assert!(!shown.contains("secret-pw") && !shown.contains("secret-key"));
    }
}
