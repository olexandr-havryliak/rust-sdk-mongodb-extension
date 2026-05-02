// Run inside container: mongosh /scripts/data_federation_e2e.js
// Verifies $readLocalJsonl: extension root + varied stage parameters (path, maxDocuments) + $match.

const d = db.getSiblingDB("data_federation_e2e");
d.dropDatabase();
d.createCollection("c");

function num(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}

// --- Full read + downstream $match (default stage params: path only) ---
const rows = d.c
  .aggregate([
    { $readLocalJsonl: { path: "sample.ndjson" } },
    { $match: { _fed: "orders" } },
  ])
  .toArray();

if (rows.length !== 2) {
  print("DATA_FED_BAD_LEN", rows.length);
  quit(40);
}
if (rows[0]._fed !== "orders" || num(rows[0].id) !== 1 || rows[0].sku !== "alpha") {
  print("DATA_FED_BAD_ROW0", JSON.stringify(rows[0]));
  quit(41);
}

// --- maxDocuments: 1 (cap emitted rows) ---
const cap1 = d.c.aggregate([{ $readLocalJsonl: { path: "sample.ndjson", maxDocuments: 1 } }]).toArray();
if (cap1.length !== 1) {
  print("DATA_FED_MAXDOC1_LEN", cap1.length);
  quit(42);
}
if (cap1[0]._fed !== "orders" || num(cap1[0].id) !== 1) {
  print("DATA_FED_MAXDOC1_ROW", JSON.stringify(cap1[0]));
  quit(43);
}

// --- maxDocuments: 2 on a different path ---
const cap2 = d.c.aggregate([{ $readLocalJsonl: { path: "events.jsonl", maxDocuments: 2 } }]).toArray();
if (cap2.length !== 2) {
  print("DATA_FED_MAXDOC2_LEN", cap2.length);
  quit(44);
}
if (cap2[0].level !== "info" || cap2[1].level !== "error") {
  print("DATA_FED_MAXDOC2_LEVELS", JSON.stringify(cap2));
  quit(45);
}

// --- Relative path with subdirectory ---
const nested = d.c.aggregate([{ $readLocalJsonl: { path: "nested/stage_params.jsonl", maxDocuments: 10 } }]).toArray();
if (nested.length !== 2) {
  print("DATA_FED_NESTED_LEN", nested.length);
  quit(46);
}
if (nested[0].kind !== "nested" || num(nested[0].n) !== 1 || nested[1].kind !== "nested" || num(nested[1].n) !== 2) {
  print("DATA_FED_NESTED_ROWS", JSON.stringify(nested));
  quit(47);
}

// --- maxDocuments: 0 (immediate EOF; zero rows) ---
const cap0 = d.c.aggregate([{ $readLocalJsonl: { path: "sample.ndjson", maxDocuments: 0 } }]).toArray();
if (cap0.length !== 0) {
  print("DATA_FED_MAXDOC0_LEN", cap0.length);
  quit(48);
}

print("DATA_FEDERATION_E2E_OK");
