//! Example extension: **`$readLocalJsonl`** streams **JSON Lines** (one JSON object per line) from
//! the **MongoDB server host** filesystem under an **`allowedRoot`** directory configured via
//! **extension options** (not from the aggregation query).
//!
//! **Demo / proof-of-concept only** — not MongoDB Data Federation. See the example README for
//! limitations and the security model.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use bson::Document;
use extension_sdk_mongodb::{
    export_source_stage, parse_args, ExtensionError, ExtensionResult, Next, SourceStage, StageContext,
};
use serde::Deserialize;

/// Default mount in the demo Docker image (Compose bind-mounts fixtures here).
pub const DEMO_ALLOWED_ROOT: &str = "/federation-data";

/// Extension options (JSON or YAML-like lines in the extension `*.conf` blob).
#[derive(Debug, Clone)]
pub struct JsonlExtensionConfig {
    /// Directory; all `path` arguments must resolve under this tree (after canonicalization).
    pub allowed_root: PathBuf,
    pub allow_symlinks: bool,
    pub max_line_bytes: u64,
    pub max_document_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct ExtensionOptionsJson {
    /// Many `mongod` builds only forward a subset of keys in the extension manifest to
    /// `get_extension_options`; when absent, the demo image uses [`DEMO_ALLOWED_ROOT`].
    #[serde(default)]
    #[serde(rename = "allowedRoot")]
    allowed_root: Option<String>,
    #[serde(default)]
    #[serde(rename = "allowSymlinks")]
    allow_symlinks: bool,
    #[serde(default)]
    #[serde(rename = "maxLineBytes")]
    max_line_bytes: Option<u64>,
    #[serde(default)]
    #[serde(rename = "maxDocumentBytes")]
    max_document_bytes: Option<u64>,
}

/// Parse extension options from raw bytes (JSON object or simple `key: value` lines).
pub fn parse_extension_options(raw: &[u8]) -> ExtensionResult<JsonlExtensionConfig> {
    let text = std::str::from_utf8(raw)
        .map_err(|e| ExtensionError::Runtime(format!("extension options are not valid UTF-8: {e}")))?;
    let t = text.trim();
    if t.starts_with('{') {
        let j: ExtensionOptionsJson = serde_json::from_str(t).map_err(|e| {
            ExtensionError::Runtime(format!("extension options JSON: {e}"))
        })?;
        let root = j
            .allowed_root
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEMO_ALLOWED_ROOT);
        return Ok(JsonlExtensionConfig {
            allowed_root: PathBuf::from(root),
            allow_symlinks: j.allow_symlinks,
            max_line_bytes: j.max_line_bytes.unwrap_or(1_048_576).max(1),
            max_document_bytes: j.max_document_bytes.unwrap_or(16_777_216).max(1),
        });
    }
    parse_extension_options_yaml_lines(t)
}

fn parse_extension_options_yaml_lines(text: &str) -> ExtensionResult<JsonlExtensionConfig> {
    let mut allowed_root: Option<String> = None;
    let mut allow_symlinks = false;
    let mut max_line_bytes: Option<u64> = None;
    let mut max_document_bytes: Option<u64> = None;
    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        if let Some(v) = s.strip_prefix("allowedRoot:") {
            allowed_root = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = s.strip_prefix("allowSymlinks:") {
            allow_symlinks = matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes");
        } else if let Some(v) = s.strip_prefix("maxLineBytes:") {
            max_line_bytes = v.trim().parse().ok();
        } else if let Some(v) = s.strip_prefix("maxDocumentBytes:") {
            max_document_bytes = v.trim().parse().ok();
        }
    }
    // Official images often omit unknown manifest keys from the options blob; keep a
    // Docker-friendly default so `examples/data-federation` and e2e fixtures still work.
    let allowed_root = allowed_root
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEMO_ALLOWED_ROOT.to_string());
    Ok(JsonlExtensionConfig {
        allowed_root: PathBuf::from(allowed_root),
        allow_symlinks,
        max_line_bytes: max_line_bytes.unwrap_or(1_048_576).max(1),
        max_document_bytes: max_document_bytes.unwrap_or(16_777_216).max(1),
    })
}

fn demo_extension_config_fallback() -> JsonlExtensionConfig {
    JsonlExtensionConfig {
        allowed_root: PathBuf::from(DEMO_ALLOWED_ROOT),
        allow_symlinks: false,
        max_line_bytes: 1_048_576,
        max_document_bytes: 16_777_216,
    }
}

fn extension_config_from_context(ctx: &mut StageContext) -> ExtensionResult<JsonlExtensionConfig> {
    let Some(raw) = ctx.extension_options_raw() else {
        return Ok(demo_extension_config_fallback());
    };
    if raw.is_empty() {
        return Ok(demo_extension_config_fallback());
    }
    parse_extension_options(&raw)
}

/// Validate relative `path` from the stage document (no `..`, no absolute, no backslash).
pub fn validate_stage_relative_path(path: &str) -> ExtensionResult<()> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ExtensionError::FailedToParse(
            "missing required field \"path\"".into(),
        ));
    }
    if path.starts_with('/') || path.contains('\\') {
        return Err(ExtensionError::BadValue(
            "path must be a relative path (no leading slash or backslashes)".into(),
        ));
    }
    for part in path.split('/') {
        if part == ".." {
            return Err(ExtensionError::BadValue(
                "path must not contain parent directory (..) segments".into(),
            ));
        }
        if part.contains('\0') {
            return Err(ExtensionError::BadValue("path contains NUL byte".into()));
        }
    }
    Ok(())
}

/// Resolve `rel` under `allowed_root` with optional symlink rejection while walking.
pub fn resolve_under_allowed_root(
    cfg: &JsonlExtensionConfig,
    rel: &str,
) -> ExtensionResult<PathBuf> {
    validate_stage_relative_path(rel)?;
    let rel = rel.trim();
    let root = fs::canonicalize(&cfg.allowed_root).map_err(|e| {
        ExtensionError::Runtime(format!("allowedRoot is not accessible: {e}"))
    })?;
    let mut cur = root.clone();
    for part in rel.split('/') {
        if part.is_empty() {
            continue;
        }
        let next = cur.join(part);
        match fs::symlink_metadata(&next) {
            Ok(m) => {
                if m.is_symlink() && !cfg.allow_symlinks {
                    return Err(ExtensionError::BadValue(format!(
                        "symlink not allowed at {}",
                        next.display()
                    )));
                }
            }
            Err(e) => {
                return Err(ExtensionError::Runtime(format!(
                    "path not found: {} ({e})",
                    next.display()
                )));
            }
        }
        cur.push(part);
    }
    let final_path = fs::canonicalize(&cur).map_err(|e| {
        ExtensionError::Runtime(format!("could not canonicalize target path: {e}"))
    })?;
    if !final_path.starts_with(&root) {
        return Err(ExtensionError::BadValue(
            "resolved path escapes allowedRoot".into(),
        ));
    }
    Ok(final_path)
}

fn open_file_for_read(path: &Path, allow_symlinks: bool) -> ExtensionResult<File> {
    #[cfg(unix)]
    {
        if !allow_symlinks {
            use std::os::unix::fs::OpenOptionsExt;
            return OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| ExtensionError::Runtime(format!("open (no symlink follow): {e}")));
        }
    }
    File::open(path).map_err(|e| ExtensionError::Runtime(format!("open file: {e}")))
}

/// BSON-encoded size of a document (UTF-8 JSON re-serialization length is not used; BSON is authoritative).
fn bson_encoded_len(doc: &Document) -> usize {
    bson::to_vec(doc).map(|v| v.len()).unwrap_or(usize::MAX)
}

/// Parse one trimmed non-empty line into a BSON document, enforcing `max_document_bytes`.
pub fn parse_jsonl_object_line(
    line: &str,
    physical_line_no: u64,
    max_document_bytes: u64,
) -> ExtensionResult<Document> {
    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
        ExtensionError::FailedToParse(format!("line {physical_line_no}: {e}"))
    })?;
    if !v.is_object() {
        return Err(ExtensionError::BadValue(format!(
            "line {physical_line_no}: JSON value must be an object, not array/scalar"
        )));
    }
    let doc = bson::to_document(&v).map_err(|e| {
        ExtensionError::FailedToParse(format!("line {physical_line_no}: {e}"))
    })?;
    let len = bson_encoded_len(&doc);
    if len as u64 > max_document_bytes {
        return Err(ExtensionError::BadValue(format!(
            "line {physical_line_no}: document encodes to {len} bytes, exceeds maxDocumentBytes ({max_document_bytes})"
        )));
    }
    Ok(doc)
}

/// Read the next logical line (delimited by `\n`), enforcing `max_line_bytes` on the line **body**
/// (excluding the trailing `\n`). Returns `Ok(None)` on EOF before any byte.
fn read_jsonl_physical_line(
    reader: &mut BufReader<File>,
    max_line_bytes: u64,
    line_buf: &mut Vec<u8>,
) -> ExtensionResult<Option<()>> {
    line_buf.clear();
    let mut nread: u64 = 0;
    loop {
        let mut b = [0u8; 1];
        let got = reader.read(&mut b).map_err(|e| ExtensionError::Runtime(format!("read: {e}")))?;
        if got == 0 {
            if line_buf.is_empty() {
                return Ok(None);
            }
            return Ok(Some(()));
        }
        if b[0] == b'\n' {
            return Ok(Some(()));
        }
        nread = nread.saturating_add(1);
        if nread > max_line_bytes {
            return Err(ExtensionError::BadValue(format!(
                "line exceeds maxLineBytes ({max_line_bytes}) before newline"
            )));
        }
        line_buf.push(b[0]);
    }
}

#[derive(Debug, Deserialize)]
struct ReadLocalJsonlArgsSerde {
    path: String,
    #[serde(rename = "maxDocuments")]
    max_documents: Option<u64>,
}

/// Typed stage arguments (`path`, optional `maxDocuments`).
#[derive(Debug, Clone)]
pub struct ReadLocalJsonlArgs {
    pub path: String,
    pub max_documents: Option<u64>,
}

pub struct ReadLocalJsonlState {
    reader: BufReader<File>,
    line_buf: Vec<u8>,
    /// Physical line number in file (1-based) for the line currently being / last attempted.
    physical_line_no: u64,
    cfg: JsonlExtensionConfig,
    max_documents: Option<u64>,
    returned: u64,
    eof_logged: bool,
}

pub struct ReadLocalJsonl;

impl SourceStage for ReadLocalJsonl {
    const NAME: &'static str = "$readLocalJsonl";
    type Args = ReadLocalJsonlArgs;
    type State = ReadLocalJsonlState;

    fn parse(args: Document) -> ExtensionResult<Self::Args> {
        let a: ReadLocalJsonlArgsSerde = parse_args(args)?;
        let path = a.path.trim().to_string();
        if path.is_empty() {
            return Err(ExtensionError::FailedToParse(
                "missing required field \"path\"".into(),
            ));
        }
        validate_stage_relative_path(&path)?;
        Ok(ReadLocalJsonlArgs {
            path,
            max_documents: a.max_documents,
        })
    }

    fn open(args: Self::Args, ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        let cfg = extension_config_from_context(ctx)?;
        let resolved = match resolve_under_allowed_root(&cfg, &args.path) {
            Ok(p) => p,
            Err(e) => {
                ctx.log_warn(&format!(
                    "readLocalJsonl: rejected or invalid path {:?}: {}",
                    args.path, e
                ));
                return Err(e);
            }
        };
        ctx.log_info(&format!(
            "readLocalJsonl: opening {}",
            resolved.display()
        ));
        let file = open_file_for_read(&resolved, cfg.allow_symlinks)?;
        let reader = BufReader::new(file);
        Ok(ReadLocalJsonlState {
            reader,
            line_buf: Vec::new(),
            physical_line_no: 0,
            cfg,
            max_documents: args.max_documents,
            returned: 0,
            eof_logged: false,
        })
    }

    fn next(state: &mut Self::State, ctx: &mut StageContext) -> ExtensionResult<Next> {
        if let Some(limit) = state.max_documents {
            if state.returned >= limit {
                if !state.eof_logged {
                    ctx.log_info("readLocalJsonl: EOF (maxDocuments reached)");
                    state.eof_logged = true;
                }
                return Ok(Next::Eof);
            }
        }
        loop {
            match read_jsonl_physical_line(
                &mut state.reader,
                state.cfg.max_line_bytes,
                &mut state.line_buf,
            )? {
                None => {
                    if !state.eof_logged {
                        ctx.log_info("readLocalJsonl: EOF (end of file)");
                        state.eof_logged = true;
                    }
                    return Ok(Next::Eof);
                }
                Some(()) => {}
            }
            state.physical_line_no = state.physical_line_no.saturating_add(1);
            let line_no = state.physical_line_no;
            ctx.metrics()
                .inc("bytes_read", state.line_buf.len() as i64);
            ctx.metrics().inc("lines_read", 1);
            let text = std::str::from_utf8(&state.line_buf).map_err(|e| {
                ExtensionError::BadValue(format!("line {line_no}: not valid UTF-8: {e}"))
            })?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                ctx.metrics().inc("empty_lines_skipped", 1);
                continue;
            }
            match parse_jsonl_object_line(trimmed, line_no, state.cfg.max_document_bytes) {
                Ok(doc) => {
                    state.returned += 1;
                    ctx.metrics().inc("documents_returned", 1);
                    return Ok(Next::Advanced {
                        document: doc,
                        metadata: None,
                    });
                }
                Err(e) => {
                    ctx.metrics().inc("parse_errors", 1);
                    let msg = e.to_string();
                    ctx.log_error(&format!("readLocalJsonl: {msg}"));
                    return Err(e);
                }
            }
        }
    }
}

export_source_stage!(ReadLocalJsonl);

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use std::io::Write;

    fn test_cfg(root: &Path) -> JsonlExtensionConfig {
        JsonlExtensionConfig {
            allowed_root: root.to_path_buf(),
            allow_symlinks: true,
            max_line_bytes: 1024,
            max_document_bytes: 64 * 1024,
        }
    }

    #[test]
    fn validate_rejects_empty() {
        let e = validate_stage_relative_path("").unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }

    #[test]
    fn validate_rejects_absolute() {
        let e = validate_stage_relative_path("/etc/passwd").unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn validate_rejects_backslash() {
        let e = validate_stage_relative_path("a\\b").unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn validate_rejects_parent() {
        let e = validate_stage_relative_path("../x").unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn parse_missing_path() {
        let e = ReadLocalJsonl::parse(doc! {}).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }

    #[test]
    fn parse_accepts_path_and_max_documents() {
        let a = ReadLocalJsonl::parse(doc! { "path": "a.jsonl", "maxDocuments": 5i64 }).expect("parse");
        assert_eq!(a.path, "a.jsonl");
        assert_eq!(a.max_documents, Some(5));
    }

    /// Stage document parameters: `path` and optional `maxDocuments` (BSON field names).
    mod stage_extension_parameters {
        use super::*;

        #[test]
        fn path_only_max_documents_none() {
            let a = ReadLocalJsonl::parse(doc! { "path": "events.jsonl" }).expect("parse");
            assert_eq!(a.path, "events.jsonl");
            assert_eq!(a.max_documents, None);
        }

        #[test]
        fn path_trimmed() {
            let a = ReadLocalJsonl::parse(doc! { "path": "  sub/file.jsonl  " }).expect("parse");
            assert_eq!(a.path, "sub/file.jsonl");
        }

        #[test]
        fn max_documents_int32() {
            let a = ReadLocalJsonl::parse(doc! { "path": "x.jsonl", "maxDocuments": 99i32 }).expect("parse");
            assert_eq!(a.max_documents, Some(99));
        }

        #[test]
        fn max_documents_negative_rejected() {
            let e = ReadLocalJsonl::parse(doc! { "path": "x.jsonl", "maxDocuments": -1i64 }).unwrap_err();
            assert!(matches!(e, ExtensionError::FailedToParse(_)));
        }

        #[test]
        fn max_documents_zero_allowed() {
            let a = ReadLocalJsonl::parse(doc! { "path": "x.jsonl", "maxDocuments": 0i64 }).expect("parse");
            assert_eq!(a.max_documents, Some(0));
        }

        #[test]
        fn path_wrong_type_fails() {
            let e = ReadLocalJsonl::parse(doc! { "path": 1i32 }).unwrap_err();
            assert!(matches!(e, ExtensionError::FailedToParse(_)));
        }

        #[test]
        fn empty_path_after_trim_fails() {
            let e = ReadLocalJsonl::parse(doc! { "path": "   " }).unwrap_err();
            assert!(matches!(e, ExtensionError::FailedToParse(_)));
        }

        #[test]
        fn extra_unknown_keys_ignored() {
            let a = ReadLocalJsonl::parse(doc! {
                "path": "a.jsonl",
                "maxDocuments": 3i64,
                "probe": "ignored",
            })
            .expect("parse");
            assert_eq!(a.path, "a.jsonl");
            assert_eq!(a.max_documents, Some(3));
        }
    }

    /// Extension manifest / options blob (`allowedRoot`, `allowSymlinks`, `maxLineBytes`, `maxDocumentBytes`).
    mod extension_options_parameters {
        use super::*;

        #[test]
        fn json_partial_fields_use_defaults_for_limits() {
            let j = r#"{"allowedRoot":"/opt/data"}"#;
            let c = parse_extension_options(j.as_bytes()).expect("parse");
            assert_eq!(c.allowed_root, PathBuf::from("/opt/data"));
            assert!(!c.allow_symlinks);
            assert_eq!(c.max_line_bytes, 1_048_576);
            assert_eq!(c.max_document_bytes, 16_777_216);
        }

        #[test]
        fn json_allow_symlinks_true() {
            let j = r#"{"allowedRoot":"/tmp","allowSymlinks":true}"#;
            let c = parse_extension_options(j.as_bytes()).expect("parse");
            assert!(c.allow_symlinks);
        }

        #[test]
        fn json_max_line_and_doc_bytes_zero_clamp_to_one() {
            let j = r#"{"allowedRoot":"/tmp","maxLineBytes":0,"maxDocumentBytes":0}"#;
            let c = parse_extension_options(j.as_bytes()).expect("parse");
            assert_eq!(c.max_line_bytes, 1);
            assert_eq!(c.max_document_bytes, 1);
        }

        #[test]
        fn json_whitespace_trimmed_allowed_root() {
            let j = r#"{"allowedRoot":"  /tmp/x  "}"#;
            let c = parse_extension_options(j.as_bytes()).expect("parse");
            assert_eq!(c.allowed_root, PathBuf::from("/tmp/x"));
        }

        #[test]
        fn json_invalid_syntax_runtime_error() {
            let e = parse_extension_options(br"{ not json").unwrap_err();
            assert!(matches!(e, ExtensionError::Runtime(_)));
        }

        #[test]
        fn yaml_allow_symlinks_yes_and_one() {
            for y in [
                "allowedRoot: /a\nallowSymlinks: yes\n",
                "allowedRoot: /a\nallowSymlinks: 1\n",
            ] {
                let c = parse_extension_options(y.as_bytes()).expect("parse");
                assert!(c.allow_symlinks, "{y}");
            }
        }

        #[test]
        fn yaml_commented_and_unknown_lines() {
            let y = "# comment\nsharedLibraryPath: /lib.so\nallowedRoot: /mnt/ro\n# another\nmaxLineBytes: 2048\n";
            let c = parse_extension_options(y.as_bytes()).expect("parse");
            assert_eq!(c.allowed_root, PathBuf::from("/mnt/ro"));
            assert_eq!(c.max_line_bytes, 2048);
        }

        #[test]
        fn yaml_quoted_allowed_root() {
            let y = "allowedRoot: \"/var/lib/x\"\n";
            let c = parse_extension_options(y.as_bytes()).expect("parse");
            assert_eq!(c.allowed_root, PathBuf::from("/var/lib/x"));
        }

        #[test]
        fn extension_options_invalid_utf8() {
            let raw: &[u8] = &[0xff, 0xfe, 0xfd];
            let e = parse_extension_options(raw).unwrap_err();
            assert!(matches!(e, ExtensionError::Runtime(s) if s.contains("UTF-8")));
        }
    }

    #[test]
    fn resolve_accepts_nested_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg = test_cfg(tmp.path());
        let sub = tmp.path().join("a").join("b.jsonl");
        fs::create_dir_all(sub.parent().unwrap()).expect("mkdir");
        fs::write(&sub, "{}\n").expect("write");
        let got = resolve_under_allowed_root(&cfg, "a/b.jsonl").expect("resolve");
        assert_eq!(got, sub.canonicalize().unwrap());
    }

    #[test]
    fn parse_jsonl_object_line_valid() {
        let d = parse_jsonl_object_line(r#"{"x":1}"#, 1, 4096).expect("ok");
        assert_eq!(d.get_i64("x").ok(), Some(1));
    }

    #[test]
    fn parse_jsonl_invalid_json_failed_to_parse() {
        let e = parse_jsonl_object_line("{not json", 3, 4096).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(s) if s.contains("line 3")));
    }

    #[test]
    fn parse_jsonl_non_object_bad_value() {
        let e = parse_jsonl_object_line("[1]", 2, 4096).unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(s) if s.contains("line 2")));
    }

    #[test]
    fn parse_jsonl_oversized_document() {
        let big = format!(r#"{{"k":"{}"}}"#, "x".repeat(50_000));
        let e = parse_jsonl_object_line(&big, 1, 100).unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn parse_extension_options_json_roundtrip() {
        let j = r#"{"allowedRoot":"/tmp/x","allowSymlinks":false,"maxLineBytes":128,"maxDocumentBytes":256}"#;
        let c = parse_extension_options(j.as_bytes()).expect("parse");
        assert_eq!(c.allowed_root, PathBuf::from("/tmp/x"));
        assert!(!c.allow_symlinks);
        assert_eq!(c.max_line_bytes, 128);
        assert_eq!(c.max_document_bytes, 256);
    }

    #[test]
    fn parse_extension_options_yaml_lines() {
        let y = "allowedRoot: /data/demo\nallowSymlinks: true\nmaxLineBytes: 512\nmaxDocumentBytes: 1024\n";
        let c = parse_extension_options(y.as_bytes()).expect("parse");
        assert_eq!(c.allowed_root, PathBuf::from("/data/demo"));
        assert!(c.allow_symlinks);
        assert_eq!(c.max_line_bytes, 512);
        assert_eq!(c.max_document_bytes, 1024);
    }

    #[test]
    fn parse_extension_options_yaml_only_shared_library_path_defaults_allowed_root() {
        let y = "sharedLibraryPath: /usr/local/lib/mongo-extensions/libdata_federation_extension.so\n";
        let c = parse_extension_options(y.as_bytes()).expect("parse");
        assert_eq!(c.allowed_root, PathBuf::from(DEMO_ALLOWED_ROOT));
        assert!(!c.allow_symlinks);
    }

    #[test]
    fn parse_extension_options_json_empty_object_uses_demo_defaults() {
        let c = parse_extension_options(b"{}\n").expect("parse");
        assert_eq!(c.allowed_root, PathBuf::from(DEMO_ALLOWED_ROOT));
        assert_eq!(c.max_line_bytes, 1_048_576);
        assert_eq!(c.max_document_bytes, 16_777_216);
    }

    #[test]
    fn read_until_limit_max_documents_via_manual_iteration() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("m.jsonl");
        fs::write(&p, "{\"a\":1}\n\n{\"a\":2}\n{\"a\":3}\n").expect("w");
        let f = File::open(&p).expect("open");
        let mut reader = BufReader::new(f);
        let mut buf = Vec::new();
        let cfg = test_cfg(tmp.path());
        let max_docs = 2u64;
        let mut out = 0u64;
        let mut line_no = 0u64;
        while out < max_docs {
            line_no += 1;
            match read_jsonl_physical_line(&mut reader, cfg.max_line_bytes, &mut buf).expect("rl") {
                None => break,
                Some(()) => {}
            }
            let t = std::str::from_utf8(&buf).expect("utf8").trim();
            if t.is_empty() {
                continue;
            }
            parse_jsonl_object_line(t, line_no, cfg.max_document_bytes).expect("parse line");
            out += 1;
        }
        assert_eq!(out, 2);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_when_disallowed() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().expect("tempdir");
        let real = tmp.path().join("real.jsonl");
        fs::write(&real, "{}\n").expect("w");
        let link = tmp.path().join("via.jsonl");
        symlink(&real, &link).expect("symlink");
        let cfg = JsonlExtensionConfig {
            allowed_root: tmp.path().to_path_buf(),
            allow_symlinks: false,
            max_line_bytes: 1024,
            max_document_bytes: 4096,
        };
        let e = resolve_under_allowed_root(&cfg, "via.jsonl").unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn max_line_bytes_enforced_by_read_jsonl_physical_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("long.jsonl");
        let mut f = std::fs::File::create(&p).expect("c");
        f.write_all(b"x").expect("w");
        for _ in 0..2000 {
            f.write_all(b"y").expect("w");
        }
        f.write_all(b"\n").expect("nl");
        drop(f);
        let f = File::open(&p).expect("open");
        let mut reader = BufReader::new(f);
        let mut buf = Vec::new();
        let e = read_jsonl_physical_line(&mut reader, 10, &mut buf).unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(s) if s.contains("maxLineBytes")));
    }
}
