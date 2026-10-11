use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use serde_json::Value;

struct AwsAuth {
    region: String,
    credentials: Credentials,
    allow_http: bool,
}

impl AwsAuth {
    fn load(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let Some(region) = get("OPENSEARCH_AWS_REGION") else {
            return Ok(None);
        };
        if region.is_empty()
            || !region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("OPENSEARCH_AWS_REGION must be a valid region name".into());
        }
        let required = |name| {
            get(name)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| format!("{name} is required for AWS signing"))
        };
        let credentials = Credentials::new(
            required("AWS_ACCESS_KEY_ID")?,
            required("AWS_SECRET_ACCESS_KEY")?,
            get("AWS_SESSION_TOKEN").filter(|v| !v.is_empty()),
            None,
            "extension-env",
        );
        Ok(Some(Self {
            region,
            credentials,
            allow_http: get("OPENSEARCH_AWS_ALLOW_HTTP").as_deref() == Some("true"),
        }))
    }

    fn headers(
        &self,
        url: &str,
        body: &[u8],
        time: SystemTime,
    ) -> Result<Vec<(String, String)>, String> {
        if !url.starts_with("https://") && !(self.allow_http && url.starts_with("http://")) {
            return Err("AWS OpenSearch requires HTTPS; HTTP is allowed only with the explicit local override".into());
        }
        let identity = self.credentials.clone().into();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("es")
            .time(time)
            .settings(SigningSettings::default())
            .build()
            .map_err(|_| "Invalid AWS signing parameters")?
            .into();
        let request = SignableRequest::new(
            "POST",
            url,
            [("content-type", "application/json")].into_iter(),
            SignableBody::Bytes(body),
        )
        .map_err(|_| "Invalid AWS request URL")?;
        let (instructions, _) = sign(request, &params)
            .map_err(|_| "AWS signing failed")?
            .into_parts();
        Ok(instructions
            .headers()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect())
    }
}

/// Send exactly the JSON bytes that were signed. Redirects are disabled so AWS
/// authorization headers cannot be forwarded to an untrusted endpoint.
pub fn post_search(url: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
    let bytes = serde_json::to_vec(body).map_err(|_| "Cannot encode OpenSearch request")?;
    let auth = AwsAuth::load(|key| std::env::var(key).ok())?;
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    let mut request = agent.post(url).set("Content-Type", "application/json");
    if let Some(auth) = auth {
        for (name, value) in auth.headers(url, &bytes, SystemTime::now())? {
            request = request.set(&name, &value);
        }
    }
    let response = request.send_bytes(&bytes).map_err(|e| match e {
        ureq::Error::Status(status, _) => format!("OpenSearch HTTP {status}"),
        ureq::Error::Transport(_) => "OpenSearch transport failed".into(),
    })?;
    if response.status() != 200 {
        return Err(format!("OpenSearch HTTP {}", response.status()));
    }
    response
        .into_json()
        .map_err(|_| "Invalid OpenSearch JSON response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        time::{Duration, UNIX_EPOCH},
    };

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let values: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| values.get(key).cloned()
    }

    #[test]
    fn signing_is_opt_in_even_when_aws_credentials_exist() {
        assert!(AwsAuth::load(env(&[
            ("AWS_ACCESS_KEY_ID", "key"),
            ("AWS_SECRET_ACCESS_KEY", "secret")
        ]))
        .unwrap()
        .is_none());
    }

    #[test]
    fn missing_or_blank_credentials_fail_without_secret_disclosure() {
        for pairs in [
            vec![("OPENSEARCH_AWS_REGION", "us-east-1")],
            vec![
                ("OPENSEARCH_AWS_REGION", "us-east-1"),
                ("AWS_ACCESS_KEY_ID", "key"),
                ("AWS_SECRET_ACCESS_KEY", " "),
            ],
            vec![
                ("OPENSEARCH_AWS_REGION", "bad/region"),
                ("AWS_ACCESS_KEY_ID", "key"),
                ("AWS_SECRET_ACCESS_KEY", "do-not-print"),
            ],
        ] {
            let error = AwsAuth::load(env(&pairs))
                .err()
                .expect("invalid credentials");
            assert!(!error.contains("do-not-print"));
        }
    }

    #[test]
    fn signature_covers_body_url_region_and_temporary_token() {
        let auth = AwsAuth::load(env(&[
            ("OPENSEARCH_AWS_REGION", "us-east-1"),
            ("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE"),
            (
                "AWS_SECRET_ACCESS_KEY",
                "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            ),
            ("AWS_SESSION_TOKEN", "temporary-token"),
        ]))
        .unwrap()
        .unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let url = "https://search-example.us-east-1.es.amazonaws.com/mongodb.demo.products/_search?allow_partial_search_results=false";
        let first = auth.headers(url, b"{}", time).unwrap();
        let header = |name: &str| {
            first
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .unwrap()
                .1
                .clone()
        };
        assert!(header("authorization").contains("AKIDEXAMPLE/20231114/us-east-1/es/aws4_request"));
        // Cross-checked with Botocore's SigV4 signer for the same fixed request.
        assert_eq!(header("authorization"), "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-east-1/es/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=881ded05b80432d47b1db28724c9aef68d473d65689ccb69cfa53c07b41a3b4a");
        assert_eq!(header("x-amz-date"), "20231114T221320Z");
        assert_eq!(header("x-amz-security-token"), "temporary-token");
        assert_ne!(first, auth.headers(url, b"{\"size\":1}", time).unwrap());
        assert_ne!(
            first,
            auth.headers(&url.replace("products", "articles"), b"{}", time)
                .unwrap()
        );
        assert!(!first.iter().any(|(_, v)| v.contains("wJalrXUtnFEMI")));
    }

    #[test]
    fn aws_http_requires_explicit_local_override() {
        let auth = AwsAuth::load(env(&[
            ("OPENSEARCH_AWS_REGION", "us-east-1"),
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
        ]))
        .unwrap()
        .unwrap();
        assert!(auth
            .headers("http://opensearch:9200/_search", b"{}", UNIX_EPOCH)
            .is_err());
        let local = AwsAuth::load(env(&[
            ("OPENSEARCH_AWS_REGION", "us-east-1"),
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
            ("OPENSEARCH_AWS_ALLOW_HTTP", "true"),
        ]))
        .unwrap()
        .unwrap();
        assert!(local
            .headers("http://opensearch:9200/_search", b"{}", UNIX_EPOCH)
            .is_ok());
    }
}
