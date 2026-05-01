//! Example extension: **`$httpFetch`** performs a blocking HTTP **GET** (like `curl`) for a
//! **`url`** in the stage document and returns **one** result document on upstream **EOF with
//! zero rows** (same generator pattern as other examples).
//!
//! **Demo only:** this enables **server-side SSRF** if exposed to untrusted aggregation callers.
//! Use only in controlled environments; add allowlists, auth, and stricter caps for production.

use std::io::Read;
use std::time::Duration;

use bson::{doc, Bson, Document};
use ureq::AgentBuilder;

const DEFAULT_MAX_BYTES: i64 = 256 * 1024;
const DEFAULT_TIMEOUT_MS: i64 = 15_000;
const ABS_MAX_BYTES: i64 = 2 * 1024 * 1024;

fn url_from_args(args: &Document) -> Result<String, String> {
    match args.get("url") {
        Some(Bson::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Err(r#""url" must be a non-empty string"#.into());
            }
            if t.len() > 2048 {
                return Err("url exceeds 2048 characters".into());
            }
            let lower = t.to_ascii_lowercase();
            if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                return Err("url must start with http:// or https://".into());
            }
            Ok(t.to_string())
        }
        Some(b) => Err(format!("url must be a string, got {b:?}")),
        None => Err(r#"missing string field "url""#.into()),
    }
}

fn bson_as_i64(v: Option<&Bson>) -> Option<i64> {
    match v? {
        Bson::Int32(i) => Some(*i as i64),
        Bson::Int64(i) => Some(*i),
        Bson::Double(f) if f.is_finite() => Some(*f as i64),
        _ => None,
    }
}

fn bounded_i64_arg(args: &Document, key: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
    let v = match bson_as_i64(args.get(key)) {
        Some(x) => x,
        None => {
            if args.get(key).is_none() {
                default
            } else {
                return Err(format!("{key} must be a finite number"));
            }
        }
    };
    if v < min || v > max {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(v)
}

fn read_body_limited(mut reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut limited = (&mut reader).take((max_bytes as u64).saturating_add(1));
    limited
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    if buf.len() > max_bytes {
        return Err(format!(
            "response body exceeds maxBytes ({max_bytes}); refusing to buffer more"
        ));
    }
    Ok(buf)
}

fn http_fetch_eof(args: &Document) -> Result<Document, String> {
    let url = url_from_args(args)?;
    let max_bytes = bounded_i64_arg(
        args,
        "maxBytes",
        DEFAULT_MAX_BYTES,
        1,
        ABS_MAX_BYTES,
    )? as usize;
    let timeout_ms = bounded_i64_arg(
        args,
        "timeoutMs",
        DEFAULT_TIMEOUT_MS,
        100,
        120_000,
    )? as u64;

    let agent = AgentBuilder::new()
        .timeout(Duration::from_millis(timeout_ms))
        .build();

    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(e) => {
            return Ok(doc! {
                "httpFetch": true,
                "url": &url,
                "error": e.to_string(),
            });
        }
    };

    let status = resp.status() as i32;
    let content_type = resp
        .header("Content-Type")
        .unwrap_or("")
        .to_string();

    let body_bytes = match read_body_limited(resp.into_reader(), max_bytes) {
        Ok(b) => b,
        Err(msg) => {
            return Ok(doc! {
                "httpFetch": true,
                "url": &url,
                "status": status,
                "contentType": &content_type,
                "error": msg,
            });
        }
    };

    let body = String::from_utf8_lossy(&body_bytes).to_string();
    let n = body_bytes.len() as i64;

    Ok(doc! {
        "httpFetch": true,
        "url": url,
        "status": status,
        "contentType": content_type,
        "body": body,
        "bytes": n,
    })
}

fn http_transform(_row: &Document, _args: &Document) -> Result<Document, String> {
    Err("$httpFetch: plan upstream with zero rows (empty collection, $limit: 0, or a $match that matches nothing), then pass { url: \"https://…\" }".into())
}

extension_sdk_mongodb::export_map_transform_stage!(
    "$httpFetch",
    false,
    http_transform,
    http_fetch_eof,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_must_be_http_https() {
        let mut a = doc! {};
        a.insert("url", "ftp://x");
        assert!(url_from_args(&a).is_err());
        let mut b = doc! {};
        b.insert("url", "https://example.com/path");
        assert_eq!(url_from_args(&b).unwrap(), "https://example.com/path");
    }

    #[test]
    fn bounded_args_defaults() {
        let a = doc! { "url": "https://example.com" };
        assert_eq!(
            bounded_i64_arg(&a, "maxBytes", DEFAULT_MAX_BYTES, 1, ABS_MAX_BYTES).unwrap(),
            DEFAULT_MAX_BYTES
        );
    }
}
