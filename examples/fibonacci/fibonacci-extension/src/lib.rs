//! Example extension: generator stage **`$fibonacci: { n: <count> }`**.
//!
//! Emits documents `{ i, value }` for the Fibonacci sequence (indices `0..n`, capped at 10 000).
//! Works as the sole stage on **`aggregate: 1`** (no collection required).

use bson::{doc, Document};
use extension_sdk_mongodb::{
    export_source_stage, parse_args, ExtensionError, ExtensionResult, Next, SourceStage, StageContext,
};
use serde::Deserialize;

/// Parsed stage arguments (after BSON / serde decode and clamping).
#[derive(Debug, Clone)]
pub struct FibArgs {
    n: usize,
}

#[derive(Debug, Deserialize)]
struct FibonacciArgs {
    n: u64,
}

/// Generator state: current index and next two Fibonacci values.
#[derive(Debug)]
pub struct FibState {
    i: usize,
    n: usize,
    a: i64,
    b: i64,
}

pub struct Fibonacci;

impl SourceStage for Fibonacci {
    const NAME: &'static str = "$fibonacci";
    type Args = FibArgs;
    type State = FibState;

    fn parse(args: Document) -> ExtensionResult<Self::Args> {
        let FibonacciArgs { n } = parse_args(args)?;
        let n = usize::try_from(n).unwrap_or(usize::MAX).min(10_000);
        Ok(FibArgs { n })
    }

    fn open(args: Self::Args, _ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        if args.n == 0 {
            return Ok(FibState {
                i: 0,
                n: 0,
                a: 0,
                b: 0,
            });
        }
        Ok(FibState {
            i: 0,
            n: args.n,
            a: 0,
            b: 1,
        })
    }

    fn next(state: &mut Self::State, ctx: &mut StageContext) -> ExtensionResult<Next> {
        if state.i >= state.n {
            return Ok(Next::Eof);
        }
        let value = state.a;
        let out = doc! { "i": state.i as i64, "value": value };
        state.i += 1;
        let nb = state.a.saturating_add(state.b);
        state.a = state.b;
        state.b = nb;
        ctx.metrics().inc("rows_out", 1);
        Ok(Next::Advanced {
            document: out,
            metadata: None,
        })
    }
}

export_source_stage!(Fibonacci);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_invalid_n_type_is_failed_to_parse() {
        let e = Fibonacci::parse(doc! { "n": "x" }).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }

    #[test]
    fn parse_missing_n_is_failed_to_parse() {
        let e = Fibonacci::parse(doc! {}).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }

    #[test]
    fn parse_negative_n_is_failed_to_parse() {
        let e = Fibonacci::parse(doc! { "n": -1 }).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }

    #[test]
    fn parse_clamps_over_10k() {
        let args = Fibonacci::parse(doc! { "n": 50_000i64 }).expect("parse");
        assert_eq!(args.n, 10_000);
    }

    #[test]
    fn parse_and_emit_ten() {
        let args = Fibonacci::parse(doc! { "n": 10 }).expect("parse");
        let mut st = Fibonacci::open(args, &mut StageContext::new()).expect("open");
        let mut rows = Vec::new();
        let mut ctx = StageContext::new();
        loop {
            match Fibonacci::next(&mut st, &mut ctx).expect("next") {
                Next::Eof => break,
                Next::Advanced { document, .. } => rows.push(document),
            }
        }
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0], doc! { "i": 0i64, "value": 0i64 });
        assert_eq!(rows[1], doc! { "i": 1i64, "value": 1i64 });
        assert_eq!(rows[2], doc! { "i": 2i64, "value": 1i64 });
        assert_eq!(rows[3], doc! { "i": 3i64, "value": 2i64 });
    }

    #[test]
    fn n_zero_emits_nothing() {
        let args = Fibonacci::parse(doc! { "n": 0 }).expect("parse");
        let mut st = Fibonacci::open(args, &mut StageContext::new()).expect("open");
        let mut ctx = StageContext::new();
        assert!(matches!(
            Fibonacci::next(&mut st, &mut ctx).expect("next"),
            Next::Eof
        ));
    }
}
