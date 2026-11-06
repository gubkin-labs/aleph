//! Inbound webhook verification: signatures are checked on the exact body bytes before the
//! route's plan runs, in constant time.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::keys::TenantId;
use crate::manifest::{SignatureEncoding, Verify};

/// Secret values (webhook signing keys, API tokens) by name, as held by the platform.
pub trait Secrets: Send + Sync {
    /// The value of a tenant's secret, if it's set.
    fn get(&self, tenant: &TenantId, name: &str) -> Option<String>;
}

/// Secrets kept in memory, for development and tests. The same values serve every tenant.
#[derive(Debug, Default)]
pub struct MemorySecrets {
    values: RwLock<HashMap<String, String>>,
}

impl MemorySecrets {
    /// No secrets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a secret.
    pub fn set(&self, name: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut values) = self.values.write() {
            values.insert(name.into(), value.into());
        }
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, _tenant: &TenantId, name: &str) -> Option<String> {
        self.values.read().ok()?.get(name).cloned()
    }
}

/// The default tolerance for delivery timestamps, in seconds.
const DEFAULT_TOLERANCE_S: u32 = 300;

/// Why a delivery was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rejected {
    /// The secret isn't configured (a platform problem, not the sender's).
    MissingSecret(String),
    /// The signature is missing, malformed, or wrong, or the delivery is stale.
    Invalid(&'static str),
}

/// Verifies a delivery's signature over `body`. `now_ms` is the current time.
pub(crate) fn verify(
    scheme: &Verify,
    secret: Option<&str>,
    headers: &BTreeMap<String, String>,
    body: &str,
    now_ms: i64,
) -> Result<(), Rejected> {
    let secret = secret.ok_or_else(|| Rejected::MissingSecret(scheme.secret().to_owned()))?;
    let header = |name: &str| headers.get(name).map(String::as_str);
    match scheme {
        Verify::HmacSha256 {
            header: name,
            encoding,
            prefix,
            timestamp_header,
            tolerance_s,
            ..
        } => {
            let value = header(name).ok_or(Rejected::Invalid("missing signature header"))?;
            let value = match prefix {
                Some(prefix) => value
                    .strip_prefix(prefix.as_str())
                    .ok_or(Rejected::Invalid("malformed signature header"))?,
                None => value,
            };
            let signature = decode(value.trim(), *encoding)?;
            let signed = match timestamp_header {
                Some(ts_header) => {
                    let ts =
                        header(ts_header).ok_or(Rejected::Invalid("missing timestamp header"))?;
                    fresh(ts, tolerance_s.unwrap_or(DEFAULT_TOLERANCE_S), now_ms)?;
                    format!("{ts}.{body}")
                }
                None => body.to_owned(),
            };
            check(secret, signed.as_bytes(), &signature)
        }
        Verify::Stripe { tolerance_s, .. } => {
            let value =
                header("stripe-signature").ok_or(Rejected::Invalid("missing Stripe-Signature"))?;
            let mut timestamp = None;
            let mut signatures = Vec::new();
            for part in value.split(',') {
                match part.trim().split_once('=') {
                    Some(("t", t)) => timestamp = Some(t),
                    Some(("v1", sig)) => signatures.push(sig),
                    _ => {}
                }
            }
            let timestamp = timestamp.ok_or(Rejected::Invalid("malformed Stripe-Signature"))?;
            fresh(
                timestamp,
                tolerance_s.unwrap_or(DEFAULT_TOLERANCE_S),
                now_ms,
            )?;
            let signed = format!("{timestamp}.{body}");
            // Stripe sends several v1 signatures while a secret is being rolled.
            let ok = signatures.iter().any(|sig| {
                decode(sig, SignatureEncoding::Hex)
                    .is_ok_and(|s| check(secret, signed.as_bytes(), &s).is_ok())
            });
            if ok {
                Ok(())
            } else {
                Err(Rejected::Invalid("signature doesn't match"))
            }
        }
        Verify::Github { .. } => {
            let value = header("x-hub-signature-256")
                .ok_or(Rejected::Invalid("missing X-Hub-Signature-256"))?;
            let hex = value
                .strip_prefix("sha256=")
                .ok_or(Rejected::Invalid("malformed X-Hub-Signature-256"))?;
            check(
                secret,
                body.as_bytes(),
                &decode(hex, SignatureEncoding::Hex)?,
            )
        }
    }
}

fn decode(text: &str, encoding: SignatureEncoding) -> Result<Vec<u8>, Rejected> {
    let malformed = Rejected::Invalid("malformed signature");
    match encoding {
        SignatureEncoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| malformed),
        SignatureEncoding::Hex => {
            if !text.len().is_multiple_of(2) {
                return Err(malformed);
            }
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| malformed.clone()))
                .collect()
        }
    }
}

fn fresh(timestamp: &str, tolerance_s: u32, now_ms: i64) -> Result<(), Rejected> {
    let seconds: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| Rejected::Invalid("malformed timestamp"))?;
    let age_ms = now_ms.saturating_sub(seconds.saturating_mul(1000)).abs();
    if age_ms > i64::from(tolerance_s) * 1000 {
        return Err(Rejected::Invalid(
            "delivery is too old (or from the future)",
        ));
    }
    Ok(())
}

/// Compares an HMAC-SHA256 in constant time.
fn check(secret: &str, signed: &[u8], signature: &[u8]) -> Result<(), Rejected> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .map_err(|_| Rejected::Invalid("invalid secret"))?;
    mac.update(signed);
    mac.verify_slice(signature)
        .map_err(|_| Rejected::Invalid("signature doesn't match"))
}

/// Computes a hex HMAC-SHA256, for tests and the dev server's example deliveries.
pub fn sign_hex(secret: &str, signed: &str) -> String {
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(signed.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn github_signatures() {
        let scheme = Verify::Github {
            secret: "gh".into(),
        };
        let body = r#"{"action":"opened"}"#;
        let sig = format!("sha256={}", sign_hex("s3cret", body));
        let ok = headers(&[("x-hub-signature-256", &sig)]);
        assert_eq!(verify(&scheme, Some("s3cret"), &ok, body, 0), Ok(()));
        assert!(verify(&scheme, Some("other"), &ok, body, 0).is_err());
        assert!(verify(&scheme, Some("s3cret"), &ok, "{}", 0).is_err());
        assert_eq!(
            verify(&scheme, None, &ok, body, 0),
            Err(Rejected::MissingSecret("gh".into()))
        );
    }

    #[test]
    fn stripe_signatures_and_staleness() {
        let scheme = Verify::Stripe {
            secret: "stripe".into(),
            tolerance_s: None,
        };
        let body = r#"{"id":"evt_1"}"#;
        let t = 1_700_000_000_i64;
        let sig = sign_hex("whsec", &format!("{t}.{body}"));
        let h = headers(&[("stripe-signature", &format!("t={t},v1=deadbeef,v1={sig}"))]);
        assert_eq!(
            verify(&scheme, Some("whsec"), &h, body, t * 1000 + 5_000),
            Ok(())
        );
        assert_eq!(
            verify(&scheme, Some("whsec"), &h, body, t * 1000 + 600_000),
            Err(Rejected::Invalid(
                "delivery is too old (or from the future)"
            ))
        );
    }
}
