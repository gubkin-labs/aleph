//! File buckets: signed upload and download URLs, and the metadata of stored files.
//!
//! File bytes never pass through a plan. A plan asks for a signed URL, a client uploads to or
//! downloads from it, and whatever serves the URL (the platform's blob gateway, or `aleph dev`)
//! checks it with [`BlobSigner::verify`] before touching any bytes. A finished upload is
//! recorded with [`crate::Engine::complete_upload`], which stores the file's metadata and runs the
//! bucket's `upload` handlers. The URL format is defined here, so every runtime reads it the same
//! way, and the platform is free to keep the bytes wherever it likes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::effects::{OutboxItem, Pending};
use crate::keys::{Keyspace, TenantId};
use crate::manifest::{BucketDecl, Trigger};
use crate::store::{CommitError, StoreError, Transaction};
use crate::value::Value;

/// Signs and checks blob URLs for one runtime.
#[derive(Clone)]
pub struct BlobSigner {
    base_url: String,
    secret: Vec<u8>,
}

impl std::fmt::Debug for BlobSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobSigner")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// What a signed URL allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BlobAccess {
    /// Upload (`PUT`) one file.
    Upload,
    /// Download (`GET`) one file.
    Download,
}

impl BlobAccess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
        }
    }
}

/// What a verified URL allows: one file, one kind of access, until `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BlobGrant {
    /// Upload or download.
    pub access: BlobAccess,
    /// The tenant whose file it is.
    pub tenant: String,
    /// The bucket.
    pub bucket: String,
    /// The file's key.
    pub key: String,
    /// The content type an upload must have (uploads only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub content_type: Option<String>,
    /// The largest upload allowed, in bytes (uploads only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional, type = "number"))]
    pub max_bytes: Option<u64>,
    /// When the URL stops working, in milliseconds since the epoch.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub expires_at: i64,
}

/// A stored file's metadata, as plans and `upload` handlers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BlobInfo {
    /// The file's key.
    pub key: String,
    /// Its size, in bytes.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub size: u64,
    /// Its content type.
    pub content_type: String,
    /// When the upload finished, in milliseconds since the epoch.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub uploaded_at: i64,
}

impl BlobInfo {
    /// The value plans see.
    pub(crate) fn to_value(&self) -> Value {
        Value::Object(BTreeMap::from([
            ("key".to_owned(), Value::String(self.key.clone())),
            (
                "size".to_owned(),
                Value::Int(i64::try_from(self.size).unwrap_or(i64::MAX)),
            ),
            (
                "contentType".to_owned(),
                Value::String(self.content_type.clone()),
            ),
            ("uploadedAt".to_owned(), Value::Int(self.uploaded_at)),
        ]))
    }
}

impl BlobSigner {
    /// The longest base URL.
    pub const MAX_BASE_URL_LEN: usize = 512;

    /// A signer for URLs under `base_url` (like `https://files.example.com` or `/_aleph/blobs`),
    /// with `secret` (at least 16 bytes) as the signing key. Keep the secret private: anyone
    /// with it can sign URLs for any file.
    pub fn new(base_url: &str, secret: &[u8]) -> Result<Self, String> {
        let base_url = base_url.trim_end_matches('/');
        if base_url.is_empty()
            || base_url.len() > Self::MAX_BASE_URL_LEN
            || base_url.contains(['?', '#'])
            || !base_url.is_ascii()
        {
            return Err(format!(
                "the blob base URL must be 1-{} ASCII characters without ? or #",
                Self::MAX_BASE_URL_LEN
            ));
        }
        if secret.len() < 16 {
            return Err("the blob signing secret must be at least 16 bytes".into());
        }
        Ok(Self {
            base_url: base_url.to_owned(),
            secret: secret.to_vec(),
        })
    }

    /// The URL for `grant`.
    pub(crate) fn sign(&self, grant: &BlobGrant) -> String {
        let mut url = format!(
            "{}/{}/{}/{}?access={}&expires={}",
            self.base_url,
            encode(&grant.tenant),
            encode(&grant.bucket),
            encode(&grant.key),
            grant.access.as_str(),
            grant.expires_at,
        );
        if let Some(content_type) = &grant.content_type {
            url.push_str("&type=");
            url.push_str(&encode(content_type));
        }
        if let Some(max_bytes) = grant.max_bytes {
            let _ = write!(url, "&max={max_bytes}");
        }
        url.push_str("&sig=");
        url.push_str(&self.signature(grant));
        url
    }

    /// Checks a URL this signer made, and returns what it allows. Fails if it was changed, was
    /// signed with another secret, or expired before `now_ms`. Accepts the full URL or just its
    /// path and query (the part after the base URL's host).
    pub fn verify(&self, url: &str, now_ms: i64) -> Result<BlobGrant, String> {
        let (path, query) = url.split_once('?').ok_or("not a blob URL: no query")?;
        let rest = path_of(path)
            .strip_prefix(path_of(&self.base_url))
            .ok_or("not a blob URL for this runtime")?;
        let mut segments = rest.trim_start_matches('/').split('/');
        let (Some(tenant), Some(bucket), Some(key), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err("not a blob URL: expected /<tenant>/<bucket>/<key>".into());
        };
        let params: BTreeMap<&str, &str> =
            query.split('&').filter_map(|p| p.split_once('=')).collect();
        let access = match params.get("access") {
            Some(&"upload") => BlobAccess::Upload,
            Some(&"download") => BlobAccess::Download,
            _ => return Err("not a blob URL: unknown access".into()),
        };
        let expires_at = params
            .get("expires")
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or("not a blob URL: no expiry")?;
        let grant = BlobGrant {
            access,
            tenant: decode(tenant)?,
            bucket: decode(bucket)?,
            key: decode(key)?,
            content_type: params.get("type").map(|v| decode(v)).transpose()?,
            max_bytes: params
                .get("max")
                .map(|v| v.parse::<u64>().map_err(|_| "not a blob URL: bad max"))
                .transpose()?,
            expires_at,
        };
        let signature = params.get("sig").ok_or("not a blob URL: no signature")?;
        if !constant_time_eq(signature.as_bytes(), self.signature(&grant).as_bytes()) {
            return Err("the blob URL's signature doesn't match".into());
        }
        if now_ms >= expires_at {
            return Err("the blob URL has expired".into());
        }
        Ok(grant)
    }

    fn signature(&self, grant: &BlobGrant) -> String {
        let signed = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            grant.access.as_str(),
            grant.tenant,
            grant.bucket,
            grant.key,
            grant.content_type.as_deref().unwrap_or(""),
            grant.max_bytes.map(|m| m.to_string()).unwrap_or_default(),
            grant.expires_at,
        );
        let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret) else {
            return String::new();
        };
        mac.update(signed.as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .fold(String::new(), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            })
    }
}

/// Why a blob gateway refused an upload or download: an HTTP status and a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct BlobRejection {
    /// 400 (bad request), 403 (bad or expired URL), 404 (no file), 413 (too large), or 415
    /// (wrong content type).
    pub status: u16,
    /// What's wrong.
    pub message: String,
}

impl BlobRejection {
    fn new(status: u16, message: impl std::fmt::Display) -> Self {
        Self {
            status,
            message: message.to_string(),
        }
    }
}

impl super::Engine {
    /// Gives plans file storage: `uploadUrl` and `downloadUrl` sign URLs with `signer`. Without
    /// one, those operations fail.
    #[must_use]
    pub fn with_blobs(mut self, signer: BlobSigner) -> Self {
        self.blobs = Some(signer);
        self
    }

    /// Checks a blob URL this engine signed (see [`BlobSigner::verify`]), against the engine's
    /// clock.
    pub fn verify_blob_url(&self, url: &str) -> Result<BlobGrant, String> {
        let signer = self
            .blobs
            .as_ref()
            .ok_or("this runtime has no file storage")?;
        signer.verify(url, self.env.now_ms())
    }

    /// Records a finished upload for `tenant`: checks it fits the bucket (size and content type),
    /// stores its metadata, and runs the bucket's `upload` handlers after commit. Call it once
    /// the bytes are safely stored. Uploading the same key again replaces the file.
    pub fn complete_upload(
        &self,
        tenant: &TenantId,
        bucket: &str,
        key: &str,
        size: u64,
        content_type: &str,
    ) -> Result<BlobInfo, String> {
        let manifest = self.manifest.manifest();
        let decl = manifest
            .buckets
            .get(bucket)
            .ok_or_else(|| format!("there's no bucket {bucket:?}"))?;
        check_key(key)?;
        if size > decl.max_bytes {
            return Err(format!(
                "the file is {size} bytes, over bucket {bucket:?}'s limit of {}",
                decl.max_bytes
            ));
        }
        if content_type.len() > BucketDecl::MAX_CONTENT_TYPE_LEN as usize
            || !decl.allows(content_type)
        {
            return Err(format!(
                "bucket {bucket:?} doesn't accept {content_type:?} files"
            ));
        }
        let keys = Keyspace::new(tenant);
        let env = self.env.as_ref();
        let now = env.now_ms();
        let info = BlobInfo {
            key: key.to_owned(),
            size,
            content_type: content_type.trim().to_ascii_lowercase(),
            uploaded_at: now,
        };
        let bytes = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
        let payload = match info.to_value() {
            Value::Object(fields) => fields,
            _ => BTreeMap::new(),
        };
        for _ in 0..=self.limits.max_conflict_retries {
            let mut tx = self.store.begin();
            tx.put(keys.blob(bucket, key), bytes.clone());
            let runs = manifest
                .handlers
                .iter()
                .filter(|h| matches!(&h.trigger, Trigger::Upload { bucket: b } if b == bucket))
                .map(|h| Pending {
                    delay_ms: 0,
                    item: OutboxItem::Run {
                        handler: h.id.clone(),
                        payload: payload.clone(),
                        attempt: 0,
                        job: None,
                        // An upload starts a chain, like a request does.
                        chain: 1,
                        turns: 0,
                    },
                })
                .collect();
            super::process::write_outbox(&mut *tx, &keys, env, now, runs)
                .map_err(|e| e.to_string())?;
            match tx.commit() {
                Ok(()) => return Ok(info),
                Err(CommitError::Conflict) => {}
                Err(CommitError::Store(e)) => return Err(e.to_string()),
            }
        }
        Err("the upload kept conflicting with other writes; try again".into())
    }

    /// What a blob gateway does with an upload to a signed URL, before and after storing its
    /// bytes: checks the URL allows uploading this file (signature, expiry, content type, and
    /// size), then [`Self::complete_upload`]s it. Store the bytes only if [`Self::check_upload`]
    /// passes, then call this.
    pub fn receive_upload(
        &self,
        url: &str,
        size: u64,
        content_type: &str,
    ) -> Result<(TenantId, BlobInfo), BlobRejection> {
        let grant = self.check_upload(url, size, content_type)?;
        let tenant = TenantId::new(&grant.tenant).map_err(|e| BlobRejection::new(400, e))?;
        let info = self
            .complete_upload(&tenant, &grant.bucket, &grant.key, size, content_type)
            .map_err(|e| BlobRejection::new(400, e))?;
        Ok((tenant, info))
    }

    /// Checks an upload against its signed URL without recording anything: the gateway calls
    /// it before accepting the bytes.
    pub fn check_upload(
        &self,
        url: &str,
        size: u64,
        content_type: &str,
    ) -> Result<BlobGrant, BlobRejection> {
        let grant = self
            .verify_blob_url(url)
            .map_err(|e| BlobRejection::new(403, e))?;
        if grant.access != BlobAccess::Upload {
            return Err(BlobRejection::new(
                403,
                "this URL is for downloading, not uploading",
            ));
        }
        let expected = grant.content_type.as_deref().unwrap_or_default();
        if !content_type.trim().eq_ignore_ascii_case(expected) {
            return Err(BlobRejection::new(
                415,
                format!("this URL accepts {expected:?} files, not {content_type:?}"),
            ));
        }
        if grant.max_bytes.is_some_and(|max| size > max) {
            return Err(BlobRejection::new(
                413,
                format!(
                    "the file is {size} bytes, over the limit of {}",
                    grant.max_bytes.unwrap_or_default()
                ),
            ));
        }
        Ok(grant)
    }

    /// What a blob gateway does with a download: checks the signed URL and finds the file.
    /// Serve its bytes with the returned content type.
    pub fn authorize_download(&self, url: &str) -> Result<(TenantId, BlobInfo), BlobRejection> {
        let grant = self
            .verify_blob_url(url)
            .map_err(|e| BlobRejection::new(403, e))?;
        if grant.access != BlobAccess::Download {
            return Err(BlobRejection::new(
                403,
                "this URL is for uploading, not downloading",
            ));
        }
        let tenant = TenantId::new(&grant.tenant).map_err(|e| BlobRejection::new(400, e))?;
        let info = self
            .blob(&tenant, &grant.bucket, &grant.key)
            .ok_or_else(|| BlobRejection::new(404, "there's no file at this key"))?;
        Ok((tenant, info))
    }

    /// A stored file's metadata, or `None` if there's no such file. For whatever serves blob
    /// downloads, to set the response's content type and size.
    pub fn blob(&self, tenant: &TenantId, bucket: &str, key: &str) -> Option<BlobInfo> {
        let mut tx = self.store.begin();
        let bytes = tx.get(&Keyspace::new(tenant).blob(bucket, key)).ok()??;
        serde_json::from_slice(&bytes).ok()
    }
}

/// Reads a stored file's metadata inside a plan's transaction: the info and its encoded size.
pub(crate) fn read_info(
    tx: &mut dyn Transaction,
    keys: &Keyspace,
    bucket: &str,
    key: &str,
) -> Result<Option<(BlobInfo, usize)>, StoreError> {
    let Some(bytes) = tx.get(&keys.blob(bucket, key))? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice::<BlobInfo>(&bytes)
        .ok()
        .map(|info| (info, bytes.len())))
}

/// The most bytes one file's stored metadata takes: its key, content type, and two numbers.
pub(crate) const MAX_BLOB_INFO_BYTES: u64 =
    4 * BucketDecl::MAX_KEY_LEN as u64 + 4 * BucketDecl::MAX_CONTENT_TYPE_LEN as u64 + 128;

/// Checks a file key: 1-512 characters, no control characters.
pub(crate) fn check_key(key: &str) -> Result<(), String> {
    let len = key.chars().count();
    if len == 0 || len > BucketDecl::MAX_KEY_LEN as usize {
        return Err(format!(
            "a file key must be 1-{} characters, got {len}",
            BucketDecl::MAX_KEY_LEN
        ));
    }
    if key.chars().any(char::is_control) {
        return Err("a file key can't contain control characters".into());
    }
    Ok(())
}

/// The path part of a URL: `/files` for `https://host/files`, the URL itself if it's a path.
fn path_of(url: &str) -> &str {
    match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |at| &rest[at..]),
        None => url,
    }
}

/// Percent-encodes everything but unreserved characters, so keys with `/` stay one segment.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or("not a blob URL: bad escape")?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "not a blob URL: not UTF-8".into())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> BlobSigner {
        BlobSigner::new("https://files.example.com/blobs", b"0123456789abcdef").unwrap()
    }

    fn grant() -> BlobGrant {
        BlobGrant {
            access: BlobAccess::Upload,
            tenant: "acme".into(),
            bucket: "avatars".into(),
            key: "users/ada/photo 1.png".into(),
            content_type: Some("image/png".into()),
            max_bytes: Some(5_000_000),
            expires_at: 2_000,
        }
    }

    #[test]
    fn a_signed_url_verifies_until_it_expires() {
        let url = signer().sign(&grant());
        assert!(url.starts_with(
            "https://files.example.com/blobs/acme/avatars/users%2Fada%2Fphoto%201.png?"
        ));
        assert_eq!(signer().verify(&url, 1_999), Ok(grant()));
        assert!(
            signer()
                .verify(&url, 2_000)
                .unwrap_err()
                .contains("expired")
        );
        // The gateway may see only the path and query.
        let path = url.trim_start_matches("https://files.example.com");
        assert_eq!(signer().verify(path, 1_000), Ok(grant()));
    }

    #[test]
    fn a_changed_or_foreign_url_is_rejected() {
        let url = signer().sign(&grant());
        for tampered in [
            url.replace("acme", "other"),
            url.replace("avatars", "secrets"),
            url.replace("image%2Fpng", "text%2Fhtml"),
            url.replace("max=5000000", "max=999999999"),
            url.replace("expires=2000", "expires=9999999"),
            url.replace("access=upload", "access=download"),
        ] {
            assert!(signer().verify(&tampered, 0).is_err(), "{tampered}");
        }
        let other =
            BlobSigner::new("https://files.example.com/blobs", b"another-secret-16b").unwrap();
        assert!(other.verify(&url, 0).unwrap_err().contains("signature"));
    }

    #[test]
    fn signers_reject_weak_secrets_and_odd_bases() {
        assert!(BlobSigner::new("/files", b"short").is_err());
        assert!(BlobSigner::new("/files?x=1", b"0123456789abcdef").is_err());
        assert!(BlobSigner::new("/_aleph/blobs/", b"0123456789abcdef").is_ok());
    }

    #[test]
    fn keys_are_checked() {
        assert!(check_key("a/b.png").is_ok());
        assert!(check_key("").is_err());
        assert!(check_key(&"x".repeat(513)).is_err());
        assert!(check_key("a\nb").is_err());
    }

    #[test]
    fn the_longest_url_fits_its_static_bound() {
        let long = BlobGrant {
            tenant: "t".repeat(64),
            bucket: "b".repeat(64),
            key: "😀".repeat(512),
            content_type: Some("x".repeat(255)),
            max_bytes: Some(BucketDecl::MAX_BYTES),
            expires_at: i64::MAX,
            ..grant()
        };
        let signer =
            BlobSigner::new(&format!("https://{}", "h".repeat(500)), b"0123456789abcdef").unwrap();
        assert!(signer.sign(&long).len() <= BucketDecl::MAX_URL_LEN as usize);
    }
}
