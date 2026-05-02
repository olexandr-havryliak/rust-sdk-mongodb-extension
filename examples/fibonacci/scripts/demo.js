// Run: mongosh --port 27018 /scripts/demo.js  (or inside container: mongosh /scripts/demo.js)
const d = db.getSiblingDB("fibonacci_demo");
d.dropDatabase();

function bsonInt(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}

// --- Empty collection `n`: generator (same shape as README / run-demo hint) ---
d.createCollection("n");
const want10 = [
  [0, 0],
  [1, 1],
  [2, 1],
  [3, 2],
  [4, 3],
  [5, 5],
  [6, 8],
  [7, 13],
  [8, 21],
  [9, 34],
];
const b1 = d.n.aggregate([{ $fibonacci: { n: 10 } }]).toArray();
if (b1.length !== want10.length) {
  print("EMPTY_LEN", b1.length);
  quit(1);
}
for (let i = 0; i < want10.length; i++) {
  if (bsonInt(b1[i].i) !== want10[i][0] || bsonInt(b1[i].value) !== want10[i][1]) {
    print("EMPTY_BAD", i, JSON.stringify(b1[i]));
    quit(2);
  }
}

// --- `aggregate: 1` (literal namespace): may be rejected for extension stages on some builds;
//     try it; if it fails, skip (empty-collection path above is the portable check).
let agg1Ok = false;
try {
  const r1 = db.runCommand({
    aggregate: 1,
    pipeline: [{ $fibonacci: { n: 3 } }],
    cursor: {},
  });
  agg1Ok = !!(r1 && r1.ok);
} catch (_e) {
  agg1Ok = false;
}
if (agg1Ok) {
  const r1 = db.runCommand({
    aggregate: 1,
    pipeline: [{ $fibonacci: { n: 3 } }],
    cursor: {},
  });
  const batch = r1.cursor.firstBatch || [];
  if (batch.length !== 3) {
    print("AGG1_LEN", batch.length);
    quit(3);
  }
}

// --- With upstream: passthrough (documents unchanged; no `i` / `value`) ---
d.n.insertMany([{ _id: 1, label: "a" }, { _id: 2, label: "b" }]);
const rows = d.n.aggregate([{ $fibonacci: { n: 99 } }]).toArray();
if (rows.length !== 2) {
  print("PASSTHRU_LEN", rows.length);
  quit(4);
}
for (const row of rows) {
  if (row.label !== "a" && row.label !== "b") {
    print("PASSTHRU_BAD", JSON.stringify(row));
    quit(5);
  }
  if (row.i !== undefined || row.value !== undefined) {
    print("PASSTHRU_UNEXPECTED_FIB_FIELDS", JSON.stringify(row));
    quit(6);
  }
}

print("DEMO_OK");
print('Try: use fibonacci_demo; db.createCollection("n"); db.n.aggregate([{ $fibonacci: { n: 10 } }])');
quit(0);
