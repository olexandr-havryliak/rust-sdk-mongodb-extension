//! End-to-end sample extension: stage **`$rustSdkE2e`** merges **`rustSdkE2eExtensionParam`** into
//! each document, taken from extension YAML key **`e2eExtensionParam`** (read once at
//! `initialize` via [`host::extension_options_raw`](extension_sdk_mongodb::host::extension_options_raw)).
//!
//! This exercises **parametrized extensions** (config blob from `/etc/mongo/extensions/*.conf`)
//! without depending on any product example crate.

use std::sync::OnceLock;

use bson::{Bson, Document};
use extension_sdk_mongodb::host;

static YAML_PARAM: OnceLock<String> = OnceLock::new();

/// Parse `e2eExtensionParam: <value>` lines from the extension options YAML blob (unit-testable).
fn parse_e2e_extension_param(yaml: &str) -> Option<String> {
    for line in yaml.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("e2eExtensionParam:") {
            let val = rest
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Some mongod builds pass extension options as JSON instead of YAML-like lines.
fn parse_e2e_extension_param_json(text: &str) -> Option<String> {
    let t = text.trim();
    if !t.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(t).ok()?;
    v.get("e2eExtensionParam")?
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Default used when the host omits options or uses a format we do not parse (e2e-only crate).
const DEFAULT_E2E_EXTENSION_PARAM: &str = "from_extension_yaml";

fn resolve_e2e_extension_param(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return DEFAULT_E2E_EXTENSION_PARAM.to_string();
    };
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw).trim();
    if raw.is_empty() {
        return DEFAULT_E2E_EXTENSION_PARAM.to_string();
    }
    parse_e2e_extension_param(raw)
        .or_else(|| parse_e2e_extension_param_json(raw))
        .unwrap_or_else(|| DEFAULT_E2E_EXTENSION_PARAM.to_string())
}

unsafe fn e2e_parse_extension_yaml(
    portal: *const extension_sdk_mongodb::sys::MongoExtensionHostPortal,
) -> Result<(), String> {
    let v = host::extension_options_raw(portal);
    let resolved = if v.data.is_null() || v.len == 0 {
        resolve_e2e_extension_param(None)
    } else {
        let slice = std::slice::from_raw_parts(v.data, v.len as usize);
        let raw = std::str::from_utf8(slice).ok();
        resolve_e2e_extension_param(raw)
    };
    let _ = YAML_PARAM.set(resolved);
    Ok(())
}

fn merge_yaml_field(out: &mut Document) {
    if let Some(p) = YAML_PARAM.get() {
        out.insert("rustSdkE2eExtensionParam", Bson::String(p.clone()));
    } else {
        out.insert("rustSdkE2eExtensionParam", Bson::Null);
    }
}

fn e2e_transform(row: &Document, _args: &Document) -> Result<Document, String> {
    let mut out = row.clone();
    merge_yaml_field(&mut out);
    Ok(out)
}

fn e2e_eof(_args: &Document) -> Result<Document, String> {
    let mut out = Document::new();
    merge_yaml_field(&mut out);
    Ok(out)
}

extension_sdk_mongodb::export_map_transform_stage!(
    "$rustSdkE2e",
    false,
    e2e_transform,
    e2e_eof,
    e2e_parse_extension_yaml,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_param_line() {
        let yaml = "sharedLibraryPath: /x.so\ne2eExtensionParam: hello_world\n";
        assert_eq!(
            parse_e2e_extension_param(yaml).as_deref(),
            Some("hello_world")
        );
    }

    #[test]
    fn ignores_unknown_lines() {
        assert_eq!(
            parse_e2e_extension_param("foo: bar\ne2eExtensionParam: z\n").as_deref(),
            Some("z")
        );
    }

    #[test]
    fn missing_key() {
        assert_eq!(parse_e2e_extension_param("only: shared\n"), None);
    }

    #[test]
    fn parses_param_json() {
        let j = r#"{"sharedLibraryPath":"/x.so","e2eExtensionParam":"json_value"}"#;
        assert_eq!(
            parse_e2e_extension_param_json(j).as_deref(),
            Some("json_value")
        );
    }

    #[test]
    fn resolve_defaults_when_empty_or_unparsed() {
        assert_eq!(super::resolve_e2e_extension_param(None), "from_extension_yaml");
        assert_eq!(super::resolve_e2e_extension_param(Some("")), "from_extension_yaml");
        assert_eq!(
            super::resolve_e2e_extension_param(Some("only: shared\n")),
            "from_extension_yaml"
        );
    }

    #[test]
    fn resolve_prefers_yaml_then_json() {
        assert_eq!(
            super::resolve_e2e_extension_param(Some("e2eExtensionParam: from_yaml\n")),
            "from_yaml"
        );
        assert_eq!(
            super::resolve_e2e_extension_param(Some(
                r#"{"e2eExtensionParam":"from_json"}"#
            )),
            "from_json"
        );
    }
}
