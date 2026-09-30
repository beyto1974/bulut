//! S3 (Garage or any S3 compatible store) implementation of [`BlobStore`].

use async_trait::async_trait;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream as AwsByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, Delete, ObjectIdentifier};
use aws_sdk_s3::Client;
use bytes::Bytes;

use crate::ports::blob_store::{BlobError, BlobRead, BlobStore, PartInfo};
use crate::storage_config::StorageConfig;

pub struct S3BlobStore {
    client: Client,
    bucket: String,
}

fn storage<E: std::fmt::Display>(e: E) -> BlobError {
    BlobError::Storage(e.to_string())
}

impl S3BlobStore {
    pub fn new(cfg: &StorageConfig) -> Self {
        let creds = Credentials::new(
            &cfg.s3_access_key_id,
            &cfg.s3_secret_access_key,
            None,
            None,
            "bulut-env",
        );
        let mut builder = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(Region::new(cfg.s3_region.clone()))
            .credentials_provider(creds)
            // Garage and most self-hosted stores need path-style addressing.
            .force_path_style(true)
            // The whole-object checksum does not match a ranged body, so only check when required.
            .response_checksum_validation(
                aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
            )
            .request_checksum_calculation(
                aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
            );
        if let Some(endpoint) = &cfg.s3_endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        Self {
            client: Client::from_conf(builder.build()),
            bucket: cfg.s3_bucket.clone(),
        }
    }
}

#[async_trait]
impl BlobStore for S3BlobStore {
    async fn start_multipart(&self, key: &str, content_type: &str) -> Result<String, BlobError> {
        let out = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .send()
            .await
            .map_err(storage)?;
        out.upload_id()
            .map(str::to_string)
            .ok_or_else(|| BlobError::Storage("no upload id returned".into()))
    }

    async fn put_part(
        &self,
        key: &str,
        upload_id: &str,
        number: i32,
        data: Bytes,
    ) -> Result<String, BlobError> {
        let out = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(number)
            .body(AwsByteStream::from(data))
            .send()
            .await
            .map_err(storage)?;
        out.e_tag()
            .map(str::to_string)
            .ok_or_else(|| BlobError::Storage("no etag returned".into()))
    }

    async fn list_parts(&self, key: &str, upload_id: &str) -> Result<Vec<PartInfo>, BlobError> {
        let mut parts = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let mut req = self
                .client
                .list_parts()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(upload_id);
            if let Some(m) = &marker {
                req = req.part_number_marker(m);
            }
            let out = match req.send().await {
                Ok(o) => o,
                Err(e)
                    if e.as_service_error()
                        .is_some_and(|s| s.meta().code() == Some("NoSuchUpload")) =>
                {
                    return Err(BlobError::NotFound)
                }
                Err(e) => return Err(storage(e)),
            };
            for p in out.parts() {
                parts.push(PartInfo {
                    number: p.part_number().unwrap_or_default(),
                    size: p.size().unwrap_or_default().max(0) as u64,
                    etag: p.e_tag().unwrap_or_default().to_string(),
                });
            }
            if out.is_truncated().unwrap_or(false) {
                marker = out.next_part_number_marker().map(str::to_string);
                if marker.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(parts)
    }

    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[PartInfo],
    ) -> Result<u64, BlobError> {
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(
                parts
                    .iter()
                    .map(|p| {
                        CompletedPart::builder()
                            .part_number(p.number)
                            .e_tag(&p.etag)
                            .build()
                    })
                    .collect(),
            ))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(storage)?;
        Ok(parts.iter().map(|p| p.size).sum())
    }

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), BlobError> {
        match self
            .client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(e)
                if e.as_service_error()
                    .is_some_and(|s| s.meta().code() == Some("NoSuchUpload")) =>
            {
                Ok(())
            }
            Err(e) => Err(storage(e)),
        }
    }

    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<(), BlobError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(AwsByteStream::from(data))
            .send()
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn read(&self, key: &str, range: Option<(u64, u64)>) -> Result<BlobRead, BlobError> {
        let mut req = self.client.get_object().bucket(&self.bucket).key(key);
        if let Some((start, end)) = range {
            req = req.range(format!("bytes={start}-{end}"));
        }
        let out = match req.send().await {
            Ok(o) => o,
            Err(SdkError::ServiceError(e)) if e.err().is_no_such_key() => {
                return Err(BlobError::NotFound)
            }
            Err(SdkError::ServiceError(e)) if e.raw().status().as_u16() == 416 => {
                return Err(BlobError::BadRange)
            }
            Err(e) => return Err(storage(e)),
        };
        let length = out.content_length().unwrap_or_default().max(0) as u64;
        // `Content-Range: bytes 0-99/1000` tells the real total and the range served.
        let (total, served) = match out.content_range().and_then(parse_content_range) {
            Some((s, e, t)) => (t, Some((s, e))),
            None => (length, None),
        };
        let body = out.body;
        let stream = futures_util::stream::unfold(body, |mut body| async move {
            match body.try_next().await {
                Ok(Some(chunk)) => Some((Ok(chunk), body)),
                Ok(None) => None,
                Err(e) => Some((Err(std::io::Error::other(e)), body)),
            }
        });
        Ok(BlobRead {
            total,
            range: served,
            stream: Box::pin(stream),
        })
    }

    async fn delete_many(&self, keys: &[String]) -> Result<(), BlobError> {
        for chunk in keys.chunks(1000) {
            let objects: Vec<ObjectIdentifier> = chunk
                .iter()
                .filter_map(|k| ObjectIdentifier::builder().key(k).build().ok())
                .collect();
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .quiet(true)
                .build()
                .map_err(storage)?;
            self.client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send()
                .await
                .map_err(storage)?;
        }
        Ok(())
    }
}

/// Parses `bytes 0-99/1000` into `(0, 99, 1000)`.
fn parse_content_range(h: &str) -> Option<(u64, u64, u64)> {
    let rest = h.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

#[cfg(test)]
mod tests {
    //! Integration tests against the shared dev Garage (needs the S3_* settings in `.env`).
    use super::*;
    use futures_util::StreamExt;

    fn store() -> S3BlobStore {
        dotenvy::dotenv().ok();
        S3BlobStore::new(&StorageConfig::from_env().expect("storage settings in .env"))
    }

    fn key(name: &str) -> String {
        format!("test/{}/{name}", uuid::Uuid::new_v4())
    }

    async fn collect(r: BlobRead) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = r.stream;
        while let Some(c) = s.next().await {
            out.extend_from_slice(&c.unwrap());
        }
        out
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some((0, 99, 1000)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes */1000"), None);
    }

    #[tokio::test]
    async fn put_read_range_and_delete() {
        let s = store();
        let k = key("small.txt");
        s.put(&k, Bytes::from_static(b"0123456789"), "text/plain")
            .await
            .unwrap();
        let all = s.read(&k, None).await.unwrap();
        assert_eq!((all.total, all.range), (10, None));
        assert_eq!(collect(all).await, b"0123456789");
        let part = s.read(&k, Some((2, 4))).await.unwrap();
        assert_eq!((part.total, part.range), (10, Some((2, 4))));
        assert_eq!(collect(part).await, b"234");
        assert!(matches!(
            s.read(&k, Some((50, 60))).await,
            Err(BlobError::BadRange)
        ));
        s.delete_many(&[k.clone(), key("never-existed")])
            .await
            .unwrap();
        assert!(matches!(s.read(&k, None).await, Err(BlobError::NotFound)));
    }

    #[tokio::test]
    async fn multipart_upload_resume_and_complete() {
        let s = store();
        let k = key("big.bin");
        let id = s
            .start_multipart(&k, "application/octet-stream")
            .await
            .unwrap();
        // S3 parts (except the last) must be at least 5 MiB.
        let first = Bytes::from(vec![7u8; 5 * 1024 * 1024]);
        let last = Bytes::from_static(b"tail");
        s.put_part(&k, &id, 1, first.clone()).await.unwrap();
        // A client that lost its connection asks which parts exist, then sends the rest.
        let have = s.list_parts(&k, &id).await.unwrap();
        assert_eq!(have.iter().map(|p| p.number).collect::<Vec<_>>(), [1]);
        assert_eq!(have[0].size, first.len() as u64);
        s.put_part(&k, &id, 2, last).await.unwrap();
        let parts = s.list_parts(&k, &id).await.unwrap();
        let size = s.complete_multipart(&k, &id, &parts).await.unwrap();
        assert_eq!(size, 5 * 1024 * 1024 + 4);
        let tail = s.read(&k, Some((size - 4, size - 1))).await.unwrap();
        assert_eq!(collect(tail).await, b"tail");
        s.delete_many(&[k]).await.unwrap();
    }

    #[tokio::test]
    async fn abort_is_idempotent_and_unknown_uploads_are_not_found() {
        let s = store();
        let k = key("abort.bin");
        let id = s.start_multipart(&k, "x").await.unwrap();
        s.abort_multipart(&k, &id).await.unwrap();
        s.abort_multipart(&k, &id).await.unwrap();
        assert!(matches!(
            s.list_parts(&k, &id).await,
            Err(BlobError::NotFound)
        ));
    }
}
