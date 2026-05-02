// Run: mongosh --port 27022 /scripts/demo.js  (inside container: mongosh /scripts/demo.js)
const d = db.getSiblingDB("data_federation_demo");
d.dropDatabase();
d.createCollection("n");

const rows = d.n
  .aggregate([
    { $readLocalJsonl: { path: "sample.ndjson" } },
    { $match: { _fed: "orders" } },
  ])
  .toArray();

if (rows.length !== 2) {
  print("BAD_LEN", rows.length);
  quit(1);
}
function num(x) {
  if (x == null) return null;
  if (typeof x === "number") return x;
  if (typeof x === "object" && typeof x.toNumber === "function") return x.toNumber();
  return Number(x);
}
if (rows[0]._fed !== "orders" || num(rows[0].id) !== 1 || rows[0].sku !== "alpha") {
  print("BAD_ROW0", JSON.stringify(rows[0]));
  quit(2);
}

const errs = d.n
  .aggregate([
    { $readLocalJsonl: { path: "events.jsonl", maxDocuments: 1000 } },
    { $match: { level: "error" } },
    { $limit: 10 },
  ])
  .toArray();
if (errs.length !== 2 || errs[0].level !== "error" || errs[1].level !== "error") {
  print("EVENTS_MATCH_BAD", JSON.stringify(errs));
  quit(3);
}

print("DEMO_OK");
print('Try: use data_federation_demo; db.createCollection("n"); db.n.aggregate([{ $readLocalJsonl: { path: "sample.ndjson" } }])');
quit(0);
