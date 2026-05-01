//! Property-style checks: BSON → `byte_buf::from_bson` → raw view → decode matches.

use bson::{doc, Bson, Document};
use extension_sdk_mongodb::byte_buf;
use proptest::prelude::*;

fn roundtrip_document(d: &Document) {
    let raw = byte_buf::from_bson(d).expect("encode");
    unsafe {
        let vt = (*raw).vtable;
        let v = ((*vt).get_view)(raw);
        let encoded = std::slice::from_raw_parts(v.data, v.len as usize);
        let round = Document::from_reader(encoded).expect("decode");
        assert_eq!(round, *d);
        ((*vt).destroy)(raw);
    }
}

proptest! {
    #[test]
    fn byte_buf_roundtrip_i32(v in any::<i32>()) {
        let d = doc! { "v": v };
        roundtrip_document(&d);
    }

    #[test]
    fn byte_buf_roundtrip_i64(v in any::<i64>()) {
        let d = doc! { "v": v };
        roundtrip_document(&d);
    }

    #[test]
    fn byte_buf_roundtrip_bool(b in any::<bool>()) {
        let d = doc! { "b": b };
        roundtrip_document(&d);
    }

    #[test]
    fn byte_buf_roundtrip_pseudo_random_string(len in 0usize..40usize, seed in any::<u64>()) {
        let mut s = String::new();
        let mut r = seed;
        for _ in 0..len {
            r = r.wrapping_mul(6364136223846793005).wrapping_add(1);
            s.push((b'a' + (r % 26) as u8) as char);
        }
        let d = doc! { "s": s };
        roundtrip_document(&d);
    }

    #[test]
    fn byte_buf_roundtrip_nested(
        a in any::<i32>(),
        b in any::<i32>(),
    ) {
        let d = doc! { "outer": { "a": a, "b": b } };
        roundtrip_document(&d);
    }

    #[test]
    fn byte_buf_roundtrip_small_array(
        elems in prop::collection::vec(any::<i32>(), 0..=12usize),
    ) {
        let arr: Vec<Bson> = elems.into_iter().map(Bson::Int32).collect();
        let d = doc! { "arr": arr };
        roundtrip_document(&d);
    }
}
