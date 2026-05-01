// Run: mongosh --port 27018 /scripts/demo.js  (or inside container: mongosh /scripts/demo.js)
const d = db.getSiblingDB("fibonacci_demo");
d.dropDatabase();
d.n.insertMany([{ _id: 1, label: "a" }, { _id: 2, label: "b" }]);

const rows = d.n
  .aggregate([{ $fibonacci: { n: 8 } }])
  .toArray();

if (rows.length !== 2) {
  print("EXPECTED_2_ROWS", rows.length);
  quit(1);
}
const want = [0, 1, 1, 2, 3, 5, 8, 13];
function bsonInt(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}
for (const row of rows) {
  if (!Array.isArray(row.fibonacci) || row.fibonacci.length !== want.length) {
    print("BAD_FIB", JSON.stringify(row));
    quit(2);
  }
  for (let i = 0; i < want.length; i++) {
    if (bsonInt(row.fibonacci[i]) !== want[i]) {
      print("BAD_FIB_VAL", i, row.fibonacci[i], want[i]);
      quit(3);
    }
  }
  if (bsonInt(row.fibonacci_n) !== 8) {
    print("BAD_N", row.fibonacci_n);
    quit(4);
  }
}

// Empty collection: one synthetic document (same as typical C++ generator-style $fibonacci).
d.e.drop();
const emptyRows = d.e.aggregate([{ $fibonacci: { n: 5 } }]).toArray();
if (emptyRows.length !== 1) {
  print("EMPTY_EXPECT_1_GOT", emptyRows.length);
  quit(5);
}
const want5 = [0, 1, 1, 2, 3];
for (let i = 0; i < want5.length; i++) {
  if (bsonInt(emptyRows[0].fibonacci[i]) !== want5[i]) {
    print("EMPTY_BAD_FIB", i, emptyRows[0].fibonacci[i]);
    quit(6);
  }
}
if (bsonInt(emptyRows[0].fibonacci_n) !== 5) {
  print("EMPTY_BAD_N", emptyRows[0].fibonacci_n);
  quit(7);
}

print("DEMO_OK");
quit(0);
