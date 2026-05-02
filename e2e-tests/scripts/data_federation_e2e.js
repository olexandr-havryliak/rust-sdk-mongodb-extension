// Run inside container: mongosh /scripts/data_federation_e2e.js
// Verifies $readLocalJsonl SourceStage + extension options (allowedRoot) + downstream $match.

const d = db.getSiblingDB("data_federation_e2e");
d.dropDatabase();
d.createCollection("c");

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
function num(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}
if (rows[0]._fed !== "orders" || num(rows[0].id) !== 1 || rows[0].sku !== "alpha") {
  print("DATA_FED_BAD_ROW0", JSON.stringify(rows[0]));
  quit(41);
}

print("DATA_FEDERATION_E2E_OK");
