// Run: mongosh --port 27021 /scripts/demo.js  (inside container: mongosh /scripts/demo.js)
const d = db.getSiblingDB("http_fetch_demo");
d.dropDatabase();
d.createCollection("_e");
const scratch = d.getCollection("_e");

const rows = scratch
  .aggregate([
    {
      $httpFetch: {
        url: "https://example.com/",
        maxBytes: 65536,
        timeoutMs: 15000,
      },
    },
  ])
  .toArray();

if (rows.length !== 1) {
  print("EXPECTED_1_ROW", rows.length);
  quit(1);
}
const r = rows[0];
if (!r.httpFetch) {
  print("MISSING_FLAG", JSON.stringify(r));
  quit(2);
}
if (r.error) {
  print("FETCH_ERROR", r.error);
  quit(3);
}
if (r.status !== 200) {
  print("BAD_STATUS", r.status);
  quit(4);
}
if (typeof r.body !== "string" || r.body.indexOf("Example Domain") < 0) {
  print("BAD_BODY", (r.body && r.body.slice(0, 200)) || null);
  quit(5);
}

print("DEMO_OK");
quit(0);
