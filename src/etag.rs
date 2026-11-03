// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Result, render::Options};
use axum::http::{HeaderMap, header};
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[derive(Clone)]
pub(crate) struct EtagKey(Vec<u8>);
impl EtagKey {
    pub(crate) fn new(salt: Option<&str>) -> Result<Self> {
        if let Some(salt) = salt {
            if !(32..=4096).contains(&salt.len()) {
                return Err("etag_salt must contain 32 to 4096 bytes".into());
            }
            Ok(Self(salt.as_bytes().to_vec()))
        } else {
            let mut bytes = vec![0; 32];
            getrandom::fill(&mut bytes).map_err(|_| "Cannot generate ETag salt")?;
            Ok(Self(bytes))
        }
    }
    pub(crate) fn tag(&self, chart: &str, options: &Options) -> String {
        // Options have already been validated and canonicalized. HMAC hides
        // source fingerprints and also prevents cross-mode cache confusion.
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts any key size");
        for part in [
            b"chuni-chart-rs/image-etag/v1".as_slice(),
            env!("RENDERER_FINGERPRINT").as_bytes(),
            serde_json::to_vec(options)
                .expect("Validated options serialize")
                .as_slice(),
            chart.as_bytes(),
        ] {
            mac.update(&(part.len() as u64).to_le_bytes());
            mac.update(part);
        }
        let digest: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // Semantic validator: byte-level encodings need not be identical.
        format!("W/\"{digest}\"")
    }
}

pub(crate) fn matches(headers: &HeaderMap, etag: &str) -> bool {
    let opaque = etag.strip_prefix("W/").unwrap_or(etag);
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .any(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate.strip_prefix("W/").unwrap_or(candidate) == opaque)
        })
}
pub(crate) fn wildcard(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .any(|value| value.to_str().is_ok_and(|s| s.trim() == "*"))
}
