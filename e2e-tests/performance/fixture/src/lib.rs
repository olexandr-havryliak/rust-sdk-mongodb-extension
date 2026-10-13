//! Deterministic stages for the Rust SDK for MongoDB Extensions performance tests.

use std::time::{Duration, Instant};

use bson::{doc, Bson, Document};
use extension_sdk_mongodb::stage_properties::{HostTypeRequirement, StageProperties};
use extension_sdk_mongodb::{ExtensionError, ExtensionResult, Next, SourceStage, StageContext};

#[derive(Clone, Debug)]
pub struct Config {
    pub rows: i64,
    pub payload_bytes: usize,
    pub metadata: bool,
    pub delay_ms: u64,
    pub fail_at: Option<i64>,
    pub panic_at: Option<i64>,
    pub passthrough: bool,
}

fn integer(args: &Document, key: &str, default: i64, max: i64) -> ExtensionResult<i64> {
    let value = match args.get(key) {
        None => default,
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Int64(value)) => *value,
        _ => {
            return Err(ExtensionError::BadValue(format!(
                "{key} must be an integer"
            )))
        }
    };
    if !(0..=max).contains(&value) {
        return Err(ExtensionError::BadValue(format!(
            "{key} must be between 0 and {max}"
        )));
    }
    Ok(value)
}

fn flag(args: &Document, key: &str) -> ExtensionResult<bool> {
    match args.get(key) {
        None => Ok(false),
        Some(Bson::Boolean(value)) => Ok(*value),
        _ => Err(ExtensionError::BadValue(format!("{key} must be a boolean"))),
    }
}

impl Config {
    pub fn parse(args: Document) -> ExtensionResult<Self> {
        for key in args.keys() {
            if ![
                "rows",
                "payload_bytes",
                "metadata",
                "delay_ms",
                "fail_at",
                "panic_at",
                "passthrough",
            ]
            .contains(&key.as_str())
            {
                return Err(ExtensionError::BadValue(format!(
                    "unknown fixture option {key}"
                )));
            }
        }
        Ok(Self {
            rows: integer(&args, "rows", 128, 1_000_000)?,
            payload_bytes: integer(&args, "payload_bytes", 256, 1_048_576)? as usize,
            metadata: flag(&args, "metadata")?,
            delay_ms: integer(&args, "delay_ms", 0, 60_000)? as u64,
            fail_at: args
                .contains_key("fail_at")
                .then(|| integer(&args, "fail_at", 0, 1_000_000))
                .transpose()?,
            panic_at: args
                .contains_key("panic_at")
                .then(|| integer(&args, "panic_at", 0, 1_000_000))
                .transpose()?,
            passthrough: flag(&args, "passthrough")?,
        })
    }

    fn check_row(&self, id: i64) -> ExtensionResult<()> {
        if self.panic_at == Some(id) {
            panic!("performance fixture panic at row {id}");
        }
        if self.fail_at == Some(id) {
            return Err(ExtensionError::Runtime(format!(
                "performance fixture failure at row {id}"
            )));
        }
        Ok(())
    }
}

pub struct State {
    config: Config,
    payload: String,
    next: i64,
}

pub struct PerfSource;

impl SourceStage for PerfSource {
    const NAME: &'static str = "$sdkPerfSource";
    type Args = Config;
    type State = State;

    fn parse(args: Document) -> ExtensionResult<Config> {
        Config::parse(args)
    }

    fn open(config: Config, _: &mut StageContext) -> ExtensionResult<State> {
        let payload = "x".repeat(config.payload_bytes);
        Ok(State {
            config,
            payload,
            next: 0,
        })
    }

    fn next(state: &mut State, ctx: &mut StageContext) -> ExtensionResult<Next> {
        ctx.check_interrupt()?;
        if state.next >= state.config.rows {
            return Ok(Next::Eof);
        }
        state.config.check_row(state.next)?;
        let started = Instant::now();
        let delay = Duration::from_millis(state.config.delay_ms);
        while started.elapsed() < delay {
            std::thread::sleep(
                Duration::from_millis(5).min(delay.saturating_sub(started.elapsed())),
            );
            ctx.check_interrupt()?;
        }
        let document = doc! {"_id": state.next, "payload": &state.payload};
        state.next += 1;
        ctx.metrics().inc("rows", 1);
        Ok(Next::Advanced {
            document,
            metadata: state
                .config
                .metadata
                .then(|| doc! {"$vectorSearchScore": 0.5}),
        })
    }

    fn properties() -> StageProperties {
        StageProperties {
            requires_input: false,
            ..StageProperties::source_stage_default()
        }
    }

    fn host_type() -> HostTypeRequirement {
        if cfg!(feature = "router") {
            HostTypeRequirement::Router
        } else {
            HostTypeRequirement::None
        }
    }
}

pub fn map_row(row: &Document, args: &Document) -> Result<Document, String> {
    let config = Config::parse(args.clone()).map_err(|error| error.to_string())?;
    let id = row
        .get("_id")
        .and_then(|value| value.as_i64().or_else(|| value.as_i32().map(i64::from)))
        .unwrap_or(-1);
    config.check_row(id).map_err(|error| error.to_string())?;
    let mut output = row.clone();
    if !config.passthrough {
        output.insert("mapped", true);
    }
    Ok(output)
}

#[cfg(all(feature = "source", feature = "transform"))]
compile_error!("source and transform export the same ABI symbols; build them separately");

#[cfg(feature = "source")]
fn open_erased(args: Document, ctx: &mut StageContext) -> ExtensionResult<*mut std::ffi::c_void> {
    Ok(Box::into_raw(Box::new(PerfSource::open(PerfSource::parse(args)?, ctx)?)).cast())
}

#[cfg(feature = "source")]
unsafe fn next_erased(
    state: *mut std::ffi::c_void,
    ctx: &mut StageContext,
) -> ExtensionResult<Next> {
    PerfSource::next(&mut *state.cast::<State>(), ctx)
}

#[cfg(feature = "source")]
unsafe fn drop_erased(state: *mut std::ffi::c_void) {
    drop(Box::from_raw(state.cast::<State>()));
}

pub fn source_properties() -> Document {
    let mut properties =
        PerfSource::properties().to_document_with_host_type(PerfSource::host_type());
    properties.insert("providedMetadataFields", vec!["vectorSearchScore"]);
    properties
}

#[cfg(feature = "source")]
fn expand_erased(args: Document) -> ExtensionResult<extension_sdk_mongodb::Expansion> {
    Ok(PerfSource::expand(&PerfSource::parse(args)?))
}

#[cfg(feature = "source")]
static SOURCE_OPS: extension_sdk_mongodb::SourceOps = extension_sdk_mongodb::SourceOps {
    name: PerfSource::NAME,
    expect_empty: false,
    open_from_doc: open_erased,
    drop_state: drop_erased,
    next: next_erased,
    on_extension_initialized: None,
    static_properties_doc: source_properties,
    expand_inner: expand_erased,
    merging_pipeline: None,
};

/// # Safety
/// The host provides writable version-vector storage.
#[cfg(feature = "source")]
#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension_versions(
    out: *mut extension_sdk_mongodb::sys::MongoExtensionAPIVersionVector,
) {
    extension_sdk_mongodb::version::write_supported_versions(out);
}

/// # Safety
/// Services live for the library lifetime and `out` points to writable host storage.
#[cfg(feature = "source")]
#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension(
    version: extension_sdk_mongodb::sys::MongoExtensionAPIVersion,
    services: *const extension_sdk_mongodb::sys::MongoExtensionHostServices,
    out: *mut *const extension_sdk_mongodb::sys::MongoExtension,
) -> *mut extension_sdk_mongodb::sys::MongoExtensionStatus {
    extension_sdk_mongodb::source_stage::get_source_extension_impl(
        &SOURCE_OPS,
        version,
        services,
        out,
    )
}

#[cfg(feature = "transform")]
fn transform_properties() -> Document {
    StageProperties::transform_stage_default().to_document_with_host_type(PerfSource::host_type())
}

#[cfg(feature = "transform")]
extension_sdk_mongodb::__export_map_stage_common!(
    "$sdkPerfTransform",
    false,
    map_row,
    None,
    None,
    transform_properties,
    None
);
