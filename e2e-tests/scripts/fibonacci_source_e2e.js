// Run inside container: mongosh /scripts/fibonacci_source_e2e.js
// SourceStage ($fibonacci) e2e:
//   1) No upstream     — aggregate:1 + only $fibonacci (when the server allows it)
//   2) Empty upstream  — named collection, zero documents, scan EOF → generator
//   3) Non-empty upstream — scan advances → passthrough (no Fibonacci fields)

const d = db.getSiblingDB("fibonacci_e2e");
d.dropDatabase();

function bsonInt(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}

function wantFibPairs(count) {
  const out = [];
  let a = 0,
    b = 1;
  for (let i = 0; i < count; i++) {
    out.push([i, a]);
    const nb = a + b;
    a = b;
    b = nb;
  }
  return out;
}

function assertFibBatch(batch, want, label) {
  if (batch.length !== want.length) {
    print(label + "_BAD_LEN", batch.length, want.length);
    quit(31);
  }
  for (let k = 0; k < want.length; k++) {
    const row = batch[k];
    if (bsonInt(row.i) !== want[k][0] || bsonInt(row.value) !== want[k][1]) {
      print(label + "_BAD_ROW", k, JSON.stringify(row), want[k]);
      quit(32);
    }
  }
}

// --- 1) No upstream: aggregate:1 (literal collection) + only $fibonacci ---
const want5 = wantFibPairs(5);
let agg1 = null;
try {
  agg1 = d.runCommand({
    aggregate: 1,
    pipeline: [{ $fibonacci: { n: 5 } }],
    cursor: {},
  });
} catch (e) {
  agg1 = { ok: 0, errmsg: String(e) };
}
if (agg1 && agg1.ok === 1 && agg1.cursor && Array.isArray(agg1.cursor.firstBatch)) {
  assertFibBatch(agg1.cursor.firstBatch, want5, "NO_UPSTREAM");
  print("SOURCE_STAGE_NO_UPSTREAM_OK");
} else {
  const msg = agg1 && (agg1.errmsg || agg1.codeName) ? String(agg1.errmsg || agg1.codeName) : "aggregate:1 threw or missing cursor";
  print("SOURCE_STAGE_NO_UPSTREAM_SKIPPED", msg);
}

// --- 2) Empty upstream: empty collection, first (and only) stage is $fibonacci ---
d.createCollection("c");
const want10 = wantFibPairs(10);
const batch = d.c.aggregate([{ $fibonacci: { n: 10 } }]).toArray();
assertFibBatch(batch, want10, "EMPTY_UPSTREAM");
print("SOURCE_STAGE_EMPTY_UPSTREAM_OK");

// --- 3) Non-empty upstream: documents flow through unchanged ---
d.c.insertMany([
  { _id: 1, label: "x" },
  { _id: 2, label: "y" },
]);
const passthrough = d.c.aggregate([{ $fibonacci: { n: 99 } }]).toArray();
if (passthrough.length !== 2) {
  print("NONEMPTY_UPSTREAM_BAD_LEN", passthrough.length);
  quit(33);
}
for (const row of passthrough) {
  if (row.label !== "x" && row.label !== "y") {
    print("NONEMPTY_UPSTREAM_BAD_LABEL", JSON.stringify(row));
    quit(34);
  }
  if (row.i !== undefined || row.value !== undefined) {
    print("NONEMPTY_UPSTREAM_UNEXPECTED_FIB", JSON.stringify(row));
    quit(35);
  }
}
print("SOURCE_STAGE_NONEMPTY_UPSTREAM_OK");

print("FIBONACCI_SOURCE_OK");
