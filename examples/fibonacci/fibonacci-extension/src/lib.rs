//! Example extension: stage `$fibonacci: { n: <count> }` adds a `fibonacci` array field to each document.
//!
//! When the upstream collection yields **no documents** (e.g. `aggregate` on an empty collection with
//! only this stage), emits **one** document `{ fibonacci: [...], fibonacci_n: n }`, matching a typical
//! C++ “generator” style `$fibonacci` implementation.

use bson::{doc, Bson, Document};

fn fibonacci_prefix(count: usize) -> Vec<i64> {
    match count {
        0 => vec![],
        1 => vec![0],
        _ => {
            let mut v = vec![0i64, 1];
            while v.len() < count {
                let next = v[v.len() - 2].saturating_add(v[v.len() - 1]);
                v.push(next);
            }
            v
        }
    }
}

fn n_from_args(args: &Document) -> Result<usize, String> {
    match args.get("n") {
        Some(Bson::Int32(i)) if *i >= 0 => Ok(*i as usize),
        Some(Bson::Int64(i)) if *i >= 0 => Ok(*i as usize),
        Some(Bson::Double(f)) if *f >= 0.0 && f.is_finite() && (*f - f.round()).abs() < f64::EPSILON => Ok(*f as usize),
        Some(Bson::Int32(_)) | Some(Bson::Int64(_)) | Some(Bson::Double(_)) => Err(r#""n" must be non-negative"#.into()),
        _ => Err(r#"stage requires integer field "n" (e.g. { $fibonacci: { n: 10 } })"#.into()),
    }
}

/// Shared sequence + effective `n` (after cap).
fn fib_payload(args: &Document) -> Result<(Vec<Bson>, i64), String> {
    let n = n_from_args(args)?;
    let cap = (n.min(10_000)) as i64;
    let seq = fibonacci_prefix(cap as usize);
    let arr: Vec<Bson> = seq.into_iter().map(Bson::Int64).collect();
    Ok((arr, cap))
}

/// One synthetic row when there are no input documents (empty collection / EOF before first advance).
fn fib_eof(args: &Document) -> Result<Document, String> {
    let (arr, cap) = fib_payload(args)?;
    Ok(doc! {
        "fibonacci": arr,
        "fibonacci_n": cap,
    })
}

fn fib_map(row: &Document, args: &Document) -> Result<Document, String> {
    let (arr, cap) = fib_payload(args)?;
    let mut out = row.clone();
    out.insert("fibonacci", Bson::Array(arr));
    out.insert("fibonacci_n", Bson::Int64(cap));
    Ok(out)
}

extension_sdk_mongodb::export_map_transform_stage!("$fibonacci", false, fib_map, fib_eof);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fibonacci_prefix_values() {
        assert!(fibonacci_prefix(0).is_empty());
        assert_eq!(fibonacci_prefix(1), vec![0]);
        assert_eq!(fibonacci_prefix(2), vec![0, 1]);
        assert_eq!(fibonacci_prefix(8), vec![0, 1, 1, 2, 3, 5, 8, 13]);
    }

    #[test]
    fn fib_map_adds_sequence() {
        let row = doc! { "x": 1 };
        let args = doc! { "n": 5 };
        let out = fib_map(&row, &args).expect("ok");
        assert_eq!(out.get_i32("x").expect("x"), 1);
        assert_eq!(out.get_i64("fibonacci_n").expect("n"), 5);
    }

    #[test]
    fn fib_eof_is_standalone_payload() {
        let args = doc! { "n": 4 };
        let out = fib_eof(&args).expect("ok");
        assert!(out.get("x").is_none());
        assert_eq!(out.get_i64("fibonacci_n").expect("n"), 4);
    }
}
