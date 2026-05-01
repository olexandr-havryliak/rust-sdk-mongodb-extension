//! End-to-end sample extension: stage **`$rustSdkE2e`** merges **`rustSdkE2eExtensionParam`** into
//! each document, taken from extension YAML key **`e2eExtensionParam`** (read once at
//! `initialize` via [`host::extension_options_raw`](extension_sdk_mongodb::host::extension_options_raw)).
//!
//! This exercises **parametrized extensions** (config blob from `/etc/mongo/extensions/*.conf`)
//! without depending on any product example crate.

use std::sync::OnceLock;

use bson::{doc, Bson, Document};
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

unsafe fn e2e_parse_extension_yaml(
    portal: *const extension_sdk_mongodb::sys::MongoExtensionHostPortal,
) -> Result<(), String> {
    let v = host::extension_options_raw(portal);
    if v.data.is_null() || v.len == 0 {
        return Ok(());
    }
    let raw = std::str::from_utf8(std::slice::from_raw_parts(v.data, v.len as usize))
        .map_err(|e| e.to_string())?;
    if let Some(s) = parse_e2e_extension_param(raw) {
        let _ = YAML_PARAM.set(s);
    }
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
}
