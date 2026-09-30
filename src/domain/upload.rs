use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// An upload in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub id: Uuid,
    pub session_code: String,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub part_size: i32,
    pub blob_key: String,
    pub s3_upload_id: String,
    pub client_created_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Upload {
    /// Number of parts the client has to send. An empty file still has zero parts.
    pub fn parts_total(&self) -> i32 {
        let part = i64::from(self.part_size);
        ((self.size + part - 1) / part) as i32
    }

    /// Exact size a given part must have: full parts, and a shorter last part.
    pub fn expected_part_size(&self, number: i32) -> Option<i64> {
        let total = self.parts_total();
        if number < 1 || number > total {
            return None;
        }
        let part = i64::from(self.part_size);
        if number < total {
            Some(part)
        } else {
            Some(self.size - part * i64::from(total - 1))
        }
    }
}

/// What the client needs to start or resume sending parts.
#[derive(Debug, Clone, Serialize)]
pub struct UploadStatus {
    pub upload_id: Uuid,
    pub name: String,
    pub size: i64,
    pub part_size: i32,
    pub parts_total: i32,
    /// Part numbers already stored, ascending. Send the rest.
    pub parts_received: Vec<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(size: i64, part_size: i32) -> Upload {
        Upload {
            id: Uuid::nil(),
            session_code: "k7m3q".into(),
            parent_id: None,
            name: "f".into(),
            content_type: "x".into(),
            size,
            part_size,
            blob_key: "k".into(),
            s3_upload_id: "u".into(),
            client_created_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn part_counts_round_up() {
        assert_eq!(upload(0, 10).parts_total(), 0);
        assert_eq!(upload(1, 10).parts_total(), 1);
        assert_eq!(upload(10, 10).parts_total(), 1);
        assert_eq!(upload(11, 10).parts_total(), 2);
        assert_eq!(upload(1_000_000_000, 8 * 1024 * 1024).parts_total(), 120);
    }

    #[test]
    fn expected_sizes_are_full_then_remainder() {
        let u = upload(25, 10);
        assert_eq!(u.expected_part_size(1), Some(10));
        assert_eq!(u.expected_part_size(2), Some(10));
        assert_eq!(u.expected_part_size(3), Some(5));
        assert_eq!(u.expected_part_size(0), None);
        assert_eq!(u.expected_part_size(4), None);
        assert_eq!(upload(20, 10).expected_part_size(2), Some(10));
        assert_eq!(upload(0, 10).expected_part_size(1), None);
    }
}
