use quote::quote;
use std::{env, fs, path::PathBuf};
use syn::{Fields, Item};

fn c_type(name: &str) -> String {
    let member = match name {
        "MongoExtensionByteContainerBytes" => Some(("MongoExtensionByteContainer", "bytes")),
        "MongoExtensionExpandedArrayElementUnion" => {
            Some(("MongoExtensionExpandedArrayElement", "parseOrAst"))
        }
        "MongoExtensionDPLArrayElementUnion" => Some(("MongoExtensionDPLArrayElement", "element")),
        "MongoExtensionLogMessageSeverityOrLevel" => {
            Some(("MongoExtensionLogMessage", "severityOrLevel"))
        }
        _ => None,
    };
    member.map_or_else(
        || name.into(),
        |(parent, field)| format!("__typeof__((({parent}*)0)->{field})"),
    )
}

fn c_field(name: &str) -> &str {
    match name {
        "type_" => "type",
        "severity_or_level" => "severityOrLevel",
        "parse_or_ast" => "parseOrAst",
        "parse_node" => "parseNode",
        "logical_stage" => "logicalStage",
        "database_name" => "databaseName",
        "collection_name" => "collectionName",
        "view_namespace" => "viewNamespace",
        "view_pipeline_len" => "viewPipelineLen",
        "view_pipeline" => "viewPipeline",
        "namespace_string" => "namespaceString",
        "uuid_string" => "uuidString",
        "in_router" => "inRouter",
        "result_document" => "resultDocument",
        "result_metadata" => "resultMetadata",
        "host_extensions_api_version" => "hostExtensionsAPIVersion",
        "host_mongodb_max_wire_version" => "hostMongoDBMaxWireVersion",
        _ => name,
    }
}

fn main() {
    let source = "../../../extension-sys-mongodb/src/abi.rs";
    println!("cargo:rerun-if-changed={source}");
    let ast = syn::parse_file(&fs::read_to_string(source).unwrap()).unwrap();
    let mut rust = Vec::new();
    let mut c = String::from(
        "/* Generated from Rust bindings; values are evaluated independently in C. */\n",
    );
    for item in ast.items {
        let (identifier, fields, variants) = match item {
            Item::Struct(item) => (item.ident, item.fields, Vec::new()),
            Item::Union(item) => (item.ident, Fields::Named(item.fields), Vec::new()),
            Item::Enum(item) => (
                item.ident,
                Fields::Unit,
                item.variants.into_iter().map(|v| v.ident).collect(),
            ),
            Item::Const(item) if item.ident.to_string().starts_with("MONGO") => {
                let identifier = item.ident;
                let key = identifier.to_string();
                rust.push(quote! { #key => sys::#identifier as u64, });
                c.push_str(&format!("CHECK(\"{key}\", (uint64_t){key});\n"));
                continue;
            }
            _ => continue,
        };
        let name = identifier.to_string();
        // These are opaque forward declarations in the public C header.
        if matches!(
            name.as_str(),
            "MongoExtensionPipelineRewriteContext" | "MongoExtensionPipelineDependencies"
        ) {
            continue;
        }
        let ctype = c_type(&name);
        let size = format!("{name}.sizeof");
        let align = format!("{name}.alignof");
        rust.push(quote! { #size => std::mem::size_of::<sys::#identifier>() as u64, });
        rust.push(quote! { #align => std::mem::align_of::<sys::#identifier>() as u64, });
        c.push_str(&format!(
            "CHECK(\"{size}\", sizeof({ctype}));\nCHECK(\"{align}\", _Alignof({ctype}));\n"
        ));
        for field in fields {
            let field = field.ident.unwrap();
            let key = format!("{name}.{field}");
            let cfield = c_field(&field.to_string()).to_string();
            rust.push(quote! { #key => std::mem::offset_of!(sys::#identifier, #field) as u64, });
            c.push_str(&format!("CHECK(\"{key}\", offsetof({ctype}, {cfield}));\n"));
        }
        for variant in variants {
            let key = format!("{name}.{variant}");
            rust.push(quote! { #key => sys::#identifier::#variant as u64, });
            c.push_str(&format!("CHECK(\"{key}\", (uint64_t){variant});\n"));
        }
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("layouts.rs"),
        quote! {
            fn layout_value(key: &str) -> u64 { match key { #(#rust)* _ => u64::MAX } }
        }
        .to_string(),
    )
    .unwrap();
    fs::write(out.join("layouts.h"), c).unwrap();
}
