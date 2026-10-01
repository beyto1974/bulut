//! In-memory object storage for tests and local experiments.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::ports::blob_store::{BlobError, BlobMeta, BlobRead, BlobStore, PartInfo};
use crate::ports::clock::Clock;

#[derive(Default)]
struct Inner {
    objects: HashMap<String, Bytes>,
    /// When each object was stored, by the store's clock.
    modified: HashMap<String, DateTime<Utc>>,
    clock: Option<Arc<dyn Clock>>,
    uploads: HashMap<(String, String), BTreeMap<i32, Bytes>>,
    next_id: u64,
}

#[derive(Default)]
pub struct MemoryBlobStore(Mutex<Inner>);

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that stamps objects with `clock` instead of the system time, so a test can age them.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let store = Self::default();
        store.0.lock().unwrap().clock = Some(clock);
        store
    }

    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<_> = self.0.lock().unwrap().objects.keys().cloned().collect();
        k.sort();
        k
    }

    pub fn open_uploads(&self) -> usize {
        self.0.lock().unwrap().uploads.len()
    }
}

impl Inner {
    fn store(&mut self, key: &str, data: Bytes) {
        let now = self.clock.as_ref().map_or_else(Utc::now, |c| c.now());
        self.modified.insert(key.to_string(), now);
        self.objects.insert(key.to_string(), data);
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn start_multipart(&self, key: &str, _content_type: &str) -> Result<String, BlobError> {
        let mut g = self.0.lock().unwrap();
        g.next_id += 1;
        let id = format!("up-{}", g.next_id);
        g.uploads
            .insert((key.to_string(), id.clone()), BTreeMap::new());
        Ok(id)
    }

    async fn put_part(
        &self,
        key: &str,
        upload_id: &str,
        number: i32,
        data: Bytes,
    ) -> Result<String, BlobError> {
        let mut g = self.0.lock().unwrap();
        let parts = g
            .uploads
            .get_mut(&(key.to_string(), upload_id.to_string()))
            .ok_or(BlobError::NotFound)?;
        let etag = format!("etag-{number}-{}", data.len());
        parts.insert(number, data);
        Ok(etag)
    }

    async fn list_parts(&self, key: &str, upload_id: &str) -> Result<Vec<PartInfo>, BlobError> {
        let g = self.0.lock().unwrap();
        let parts = g
            .uploads
            .get(&(key.to_string(), upload_id.to_string()))
            .ok_or(BlobError::NotFound)?;
        Ok(parts
            .iter()
            .map(|(n, d)| PartInfo {
                number: *n,
                size: d.len() as u64,
                etag: format!("etag-{n}-{}", d.len()),
            })
            .collect())
    }

    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[PartInfo],
    ) -> Result<u64, BlobError> {
        let mut g = self.0.lock().unwrap();
        let stored = g
            .uploads
            .remove(&(key.to_string(), upload_id.to_string()))
            .ok_or(BlobError::NotFound)?;
        let mut out = Vec::new();
        for p in parts {
            out.extend_from_slice(stored.get(&p.number).ok_or(BlobError::NotFound)?);
        }
        let size = out.len() as u64;
        g.store(key, Bytes::from(out));
        Ok(size)
    }

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), BlobError> {
        self.0
            .lock()
            .unwrap()
            .uploads
            .remove(&(key.to_string(), upload_id.to_string()));
        Ok(())
    }

    async fn put(&self, key: &str, data: Bytes, _content_type: &str) -> Result<(), BlobError> {
        self.0.lock().unwrap().store(key, data);
        Ok(())
    }

    async fn read(&self, key: &str, range: Option<(u64, u64)>) -> Result<BlobRead, BlobError> {
        let data = self
            .0
            .lock()
            .unwrap()
            .objects
            .get(key)
            .cloned()
            .ok_or(BlobError::NotFound)?;
        let total = data.len() as u64;
        let (slice, range) = match range {
            None => (data, None),
            Some((start, end)) => {
                if start >= total || end < start {
                    return Err(BlobError::BadRange);
                }
                let end = end.min(total - 1);
                (
                    data.slice(start as usize..=end as usize),
                    Some((start, end)),
                )
            }
        };
        Ok(BlobRead {
            total,
            range,
            stream: Box::pin(futures_util::stream::once(async move { Ok(slice) })),
        })
    }

    async fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<BlobMeta>, BlobError> {
        let g = self.0.lock().unwrap();
        let mut keys: Vec<&String> = g
            .objects
            .keys()
            .filter(|k| after.map_or(true, |a| k.as_str() > a))
            .collect();
        keys.sort();
        Ok(keys
            .into_iter()
            .take(limit)
            .map(|k| BlobMeta {
                key: k.clone(),
                modified: g.modified[k],
            })
            .collect())
    }

    async fn delete_many(&self, keys: &[String]) -> Result<(), BlobError> {
        let mut g = self.0.lock().unwrap();
        for k in keys {
            g.objects.remove(k);
            g.modified.remove(k);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    async fn body(r: BlobRead) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = r.stream;
        while let Some(chunk) = s.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        out
    }

    #[tokio::test]
    async fn multipart_roundtrip_and_resume_listing() {
        let store = MemoryBlobStore::new();
        let id = store.start_multipart("k", "text/plain").await.unwrap();
        store
            .put_part("k", &id, 2, Bytes::from_static(b"world"))
            .await
            .unwrap();
        store
            .put_part("k", &id, 1, Bytes::from_static(b"hello "))
            .await
            .unwrap();
        let parts = store.list_parts("k", &id).await.unwrap();
        assert_eq!(parts.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2]);
        let size = store.complete_multipart("k", &id, &parts).await.unwrap();
        assert_eq!(size, 11);
        assert_eq!(
            body(store.read("k", None).await.unwrap()).await,
            b"hello world"
        );
        assert_eq!(store.open_uploads(), 0);
    }

    #[tokio::test]
    async fn range_reads_clamp_and_reject() {
        let store = MemoryBlobStore::new();
        store
            .put("k", Bytes::from_static(b"0123456789"), "x")
            .await
            .unwrap();
        let r = store.read("k", Some((2, 4))).await.unwrap();
        assert_eq!((r.total, r.range), (10, Some((2, 4))));
        assert_eq!(body(r).await, b"234");
        let r = store.read("k", Some((8, 99))).await.unwrap();
        assert_eq!(r.range, Some((8, 9)));
        assert!(matches!(
            store.read("k", Some((10, 12))).await,
            Err(BlobError::BadRange)
        ));
        assert!(matches!(
            store.read("missing", None).await,
            Err(BlobError::NotFound)
        ));
    }

    #[tokio::test]
    async fn list_pages_in_key_order_with_the_clock_of_the_store() {
        let clock = Arc::new(crate::ports::clock::ManualClock::new(Utc::now()));
        let s = MemoryBlobStore::with_clock(clock.clone());
        let t0 = clock.now();
        for k in ["c", "a"] {
            s.put(k, Bytes::from_static(b"x"), "text/plain")
                .await
                .unwrap();
        }
        clock.advance(chrono::Duration::hours(1));
        s.put("b", Bytes::from_static(b"x"), "text/plain")
            .await
            .unwrap();

        let first = s.list(None, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(first[0].modified, t0);
        assert_eq!(first[1].modified, t0 + chrono::Duration::hours(1));
        let rest = s.list(Some("b"), 2).await.unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].key, "c");
        assert!(s.list(Some("c"), 2).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_many_ignores_missing_keys() {
        let store = MemoryBlobStore::new();
        store.put("a", Bytes::from_static(b"1"), "x").await.unwrap();
        store
            .delete_many(&["a".into(), "nope".into()])
            .await
            .unwrap();
        assert!(store.keys().is_empty());
    }
}
