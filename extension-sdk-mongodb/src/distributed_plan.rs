//! Ownership adapters for a source followed by host stages on the merger.

use bson::Document;

use crate::{
    error::{ExtensionError, Result},
    host, status,
    sys::*,
};

pub(crate) struct OwnedStage(Option<MongoExtensionDPLArrayElement>);

impl OwnedStage {
    pub(crate) unsafe fn logical(stage: *mut MongoExtensionLogicalAggStage) -> Self {
        Self(Some(MongoExtensionDPLArrayElement {
            type_: MongoExtensionDPLArrayElementType::kLogical,
            element: MongoExtensionDPLArrayElementUnion {
                logical_stage: stage,
            },
        }))
    }

    fn parse(stage: *mut MongoExtensionAggStageParseNode) -> Self {
        Self(Some(MongoExtensionDPLArrayElement {
            type_: MongoExtensionDPLArrayElementType::kParse,
            element: MongoExtensionDPLArrayElementUnion { parse_node: stage },
        }))
    }
}

impl Drop for OwnedStage {
    fn drop(&mut self) {
        if let Some(stage) = self.0.take() {
            unsafe {
                match stage.type_ {
                    MongoExtensionDPLArrayElementType::kParse => {
                        let p = stage.element.parse_node;
                        ((*(*p).vtable).destroy)(p);
                    }
                    MongoExtensionDPLArrayElementType::kLogical => {
                        let p = stage.element.logical_stage;
                        ((*(*p).vtable).destroy)(p);
                    }
                }
            }
        }
    }
}

#[repr(C)]
struct Container {
    base: MongoExtensionDPLArrayContainer,
    stages: Option<Vec<OwnedStage>>,
}

unsafe extern "C" fn container_destroy(p: *mut MongoExtensionDPLArrayContainer) {
    if !p.is_null() {
        drop(Box::from_raw(p.cast::<Container>()));
    }
}

unsafe extern "C" fn container_size(p: *const MongoExtensionDPLArrayContainer) -> usize {
    (*p.cast::<Container>()).stages.as_ref().map_or(0, Vec::len)
}

unsafe extern "C" fn container_transfer(
    p: *mut MongoExtensionDPLArrayContainer,
    out: *mut MongoExtensionDPLArray,
) -> *mut MongoExtensionStatus {
    if p.is_null() || out.is_null() {
        return ExtensionError::BadValue("null DPL transfer argument".into()).into_raw_status();
    }
    let this = &mut *p.cast::<Container>();
    let Some(stages) = this.stages.as_ref() else {
        return ExtensionError::BadValue("DPL container already transferred".into())
            .into_raw_status();
    };
    if (*out).size != stages.len() || (!stages.is_empty() && (*out).elements.is_null()) {
        return ExtensionError::BadValue("invalid DPL output array".into()).into_raw_status();
    }
    for (i, mut stage) in this.stages.take().unwrap().into_iter().enumerate() {
        (*out).elements.add(i).write(stage.0.take().unwrap());
    }
    status::status_ok()
}

static CONTAINER_VTABLE: MongoExtensionDPLArrayContainerVTable =
    MongoExtensionDPLArrayContainerVTable {
        destroy: container_destroy,
        size: container_size,
        transfer: container_transfer,
    };

#[repr(C)]
struct Plan {
    base: MongoExtensionDistributedPlanLogic,
    merger: Option<Box<Container>>,
    shards: Option<Box<Container>>,
    shards_extracted: bool,
}

unsafe extern "C" fn plan_destroy(p: *mut MongoExtensionDistributedPlanLogic) {
    if !p.is_null() {
        drop(Box::from_raw(p.cast::<Plan>()));
    }
}

unsafe extern "C" fn extract_shards(
    p: *mut MongoExtensionDistributedPlanLogic,
    out: *mut *mut MongoExtensionDPLArrayContainer,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let this = &mut *p.cast::<Plan>();
    if this.shards_extracted {
        return ExtensionError::BadValue("DPL shards already extracted".into()).into_raw_status();
    }
    this.shards_extracted = true;
    if let Some(container) = this.shards.take() {
        *out = Box::into_raw(container).cast();
    }
    status::status_ok()
}

unsafe extern "C" fn extract_merger(
    p: *mut MongoExtensionDistributedPlanLogic,
    out: *mut *mut MongoExtensionDPLArrayContainer,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let Some(container) = (*p.cast::<Plan>()).merger.take() else {
        return ExtensionError::BadValue("DPL merger already extracted".into()).into_raw_status();
    };
    *out = Box::into_raw(container).cast();
    status::status_ok()
}

unsafe extern "C" fn sort_pattern(
    _: *const MongoExtensionDistributedPlanLogic,
    out: *mut *mut MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    status::status_ok()
}

static PLAN_VTABLE: MongoExtensionDistributedPlanLogicVTable =
    MongoExtensionDistributedPlanLogicVTable {
        destroy: plan_destroy,
        extract_shards_pipeline: extract_shards,
        extract_merging_pipeline: extract_merger,
        get_sort_pattern: sort_pattern,
    };

pub(crate) fn build_merger(
    source: OwnedStage,
    suffix: Vec<Document>,
    suppress_input: bool,
) -> Result<*mut MongoExtensionDistributedPlanLogic> {
    let mut stages = vec![source];
    for spec in suffix {
        if spec.len() != 1 || !spec.keys().next().unwrap().starts_with('$') {
            return Err(ExtensionError::BadValue(
                "DPL suffix must contain single-operator stage documents".into(),
            ));
        }
        stages.push(OwnedStage::parse(host::create_host_parse_node(&spec)?));
    }
    let merger = Box::new(Container {
        base: MongoExtensionDPLArrayContainer {
            vtable: &CONTAINER_VTABLE,
        },
        stages: Some(stages),
    });
    // An absent shard component means an empty pipeline, not an empty stream.
    // Use a native constant-false match so mongos receives EOF without scanning data.
    let shards = if suppress_input {
        Some(Box::new(Container {
            base: MongoExtensionDPLArrayContainer {
                vtable: &CONTAINER_VTABLE,
            },
            stages: Some(vec![OwnedStage::parse(host::create_host_parse_node(
                &bson::doc! {"$match": {"$expr": false}},
            )?)]),
        }))
    } else {
        None
    };
    Ok(Box::into_raw(Box::new(Plan {
        base: MongoExtensionDistributedPlanLogic {
            vtable: &PLAN_VTABLE,
        },
        merger: Some(merger),
        shards,
        shards_extracted: false,
    }))
    .cast())
}
