//! **Blocking-stage** abstraction: buffer upstream documents, then emit one or more results after
//! the input stream ends. This models planner “blocking” behavior separately from per-row
//! **transform** or pull-based **source** generators.
//!
//! This trait is **SDK-level only** in the current release: there is no `export_blocking_stage!`
//! yet; integration with `MongoExtensionExecAggStage` would follow the same patterns as
//! [`crate::source_stage`] / [`crate::map_transform`].

use bson::Document;

use crate::error::Result;
use crate::expansion::Expansion;
use crate::stage_context::StageContext;
use crate::stage_model::StagePlan;
use crate::stage_output::Next;
use crate::stage_properties::StageProperties;

/// A stage that consumes the entire upstream sequence before emitting output rows.
///
/// Typical pattern: the host (or a future SDK executor) calls **`consume`** for each advanced
/// upstream document, then **`finish`** exactly once after upstream **EOF**. Implementations may
/// buffer in **`State`** between calls.
pub trait BlockingStage: Sized + Send + 'static {
    /// Stage key including leading `$`, must match BSON (`{ Self::NAME: <args> }`).
    const NAME: &'static str;
    /// Parsed inner object for `{ Self::NAME: <args> }`.
    type Args;
    /// Mutable state across **`consume`** / **`finish`**.
    type State;

    /// Decode stage arguments from BSON.
    fn parse(args: Document) -> Result<Self::Args>;
    /// Allocate state before the first **`consume`**.
    fn open(args: Self::Args, ctx: &mut StageContext) -> Result<Self::State>;

    /// Called once per upstream row while the stage is accepting input.
    fn consume(state: &mut Self::State, input: Document, ctx: &mut StageContext) -> Result<()>;

    /// Called once after upstream EOF; produces all output rows (often one **`Next::Advanced`**
    /// each, then the host may synthesize final EOF as needed).
    fn finish(state: &mut Self::State, ctx: &mut StageContext) -> Result<Vec<Next>>;

    /// Parse-time expansion (SDK-only execution today; reserved for future host wiring).
    ///
    /// Default: no expansion ([`Expansion::SelfStage`]).
    fn expand(_args: &Self::Args) -> Expansion {
        Expansion::SelfStage
    }

    /// Planner static properties aligned with [`StagePlan::blocking_default`](crate::stage_model::StagePlan::blocking_default).
    fn properties() -> StageProperties {
        StagePlan::blocking_default().properties
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    /// Collects `n` from each `{ n: <i32> }` input; **`finish`** emits count then sum as two docs.
    struct CountSumStage;

    struct CountSumArgs;
    struct CountSumState {
        count: i32,
        sum: i32,
        finished: bool,
    }

    impl BlockingStage for CountSumStage {
        const NAME: &'static str = "$countSumBlockingTest";
        type Args = CountSumArgs;
        type State = CountSumState;

        fn parse(args: Document) -> Result<Self::Args> {
            if !args.is_empty() {
                return Err(crate::ExtensionError::BadValue(
                    "expected empty args".into(),
                ));
            }
            Ok(CountSumArgs)
        }

        fn open(_args: Self::Args, _ctx: &mut StageContext) -> Result<Self::State> {
            Ok(CountSumState {
                count: 0,
                sum: 0,
                finished: false,
            })
        }

        fn consume(state: &mut Self::State, input: Document, _ctx: &mut StageContext) -> Result<()> {
            assert!(
                !state.finished,
                "consume must not run after finish (EOF semantics)"
            );
            let n = input.get_i32("n").unwrap_or(0);
            state.count += 1;
            state.sum += n;
            Ok(())
        }

        fn finish(state: &mut Self::State, _ctx: &mut StageContext) -> Result<Vec<Next>> {
            assert!(!state.finished, "finish must run at most once");
            state.finished = true;
            Ok(vec![
                Next::Advanced {
                    document: doc! { "kind": "count", "v": state.count },
                    metadata: None,
                },
                Next::Advanced {
                    document: doc! { "kind": "sum", "v": state.sum },
                    metadata: None,
                },
            ])
        }
    }

    #[test]
    fn blocking_expand_default_is_self_stage() {
        let a = CountSumStage::parse(doc! {}).unwrap();
        assert_eq!(CountSumStage::expand(&a), Expansion::SelfStage);
    }

    #[test]
    fn blocking_default_properties_match_blocking_stage_plan() {
        assert_eq!(
            CountSumStage::properties(),
            StagePlan::blocking_default().properties
        );
    }

    #[test]
    fn finish_after_zero_consumes_emits_zero_aggregates() {
        let mut ctx = StageContext::new();
        let mut st = CountSumStage::open(CountSumStage::parse(doc! {}).unwrap(), &mut ctx).unwrap();
        let out = CountSumStage::finish(&mut st, &mut ctx).unwrap();
        assert_eq!(out.len(), 2);
        let d0 = match &out[0] {
            Next::Advanced { document, .. } => document,
            _ => panic!("expected Advanced"),
        };
        let d1 = match &out[1] {
            Next::Advanced { document, .. } => document,
            _ => panic!("expected Advanced"),
        };
        assert_eq!(d0.get_i32("v").unwrap(), 0);
        assert_eq!(d1.get_i32("v").unwrap(), 0);
        assert!(st.finished);
    }

    #[test]
    fn consumes_all_input_then_finish_emits_outputs() {
        let mut ctx = StageContext::new();
        let mut st = CountSumStage::open(CountSumStage::parse(doc! {}).unwrap(), &mut ctx).unwrap();
        CountSumStage::consume(&mut st, doc! { "n": 2i32 }, &mut ctx).unwrap();
        CountSumStage::consume(&mut st, doc! { "n": 3i32 }, &mut ctx).unwrap();
        CountSumStage::consume(&mut st, doc! { "n": 4i32 }, &mut ctx).unwrap();
        let out = CountSumStage::finish(&mut st, &mut ctx).unwrap();
        assert_eq!(out.len(), 2);
        let d0 = match &out[0] {
            Next::Advanced { document, .. } => document,
            _ => panic!("expected Advanced"),
        };
        let d1 = match &out[1] {
            Next::Advanced { document, .. } => document,
            _ => panic!("expected Advanced"),
        };
        assert_eq!(d0.get_i32("v").unwrap(), 3);
        assert_eq!(d1.get_i32("v").unwrap(), 9);
        assert!(st.finished);
    }

    #[test]
    fn output_only_after_finish() {
        let mut ctx = StageContext::new();
        let mut st = CountSumStage::open(CountSumStage::parse(doc! {}).unwrap(), &mut ctx).unwrap();
        CountSumStage::consume(&mut st, doc! { "n": 1i32 }, &mut ctx).unwrap();
        let out = CountSumStage::finish(&mut st, &mut ctx).unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    #[should_panic(expected = "consume must not run after finish")]
    fn consume_after_upstream_eof_panics() {
        let mut ctx = StageContext::new();
        let mut st = CountSumStage::open(CountSumStage::parse(doc! {}).unwrap(), &mut ctx).unwrap();
        CountSumStage::consume(&mut st, doc! { "n": 5i32 }, &mut ctx).unwrap();
        let _ = CountSumStage::finish(&mut st, &mut ctx).unwrap();
        let _ = CountSumStage::consume(&mut st, doc! { "n": 1i32 }, &mut ctx);
    }

    #[test]
    #[should_panic(expected = "finish must run at most once")]
    fn finish_twice_panics() {
        let mut ctx = StageContext::new();
        let mut st = CountSumStage::open(CountSumStage::parse(doc! {}).unwrap(), &mut ctx).unwrap();
        let _ = CountSumStage::finish(&mut st, &mut ctx).unwrap();
        let _ = CountSumStage::finish(&mut st, &mut ctx);
    }
}
