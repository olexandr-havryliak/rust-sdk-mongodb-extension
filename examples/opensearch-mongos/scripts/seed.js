const assert = require("node:assert/strict");
const demo = db.getSiblingDB("search_demo");
for (const name of ["range", "hashed", "compound"]) {
  const documents = [
    {_id: "shared", tenant: -1, title: "Rain shell", description: "Waterproof hiking jacket for rainy mountain trails"},
    {_id: "shared", tenant: 1, title: "Trail backpack", description: "A lightweight hiking backpack for day trips"},
    {_id: ObjectId("000000000000000000000003"), tenant: NumberLong("-2"), title: "Tent", description: "Two-person camping tent with a waterproof flysheet"},
    {_id: NumberLong("4"), tenant: NumberLong("2"), title: "Sleeping bag", description: "Insulated sleeping bag for cold winter nights"},
  ];
  if (name === "compound") documents.forEach(d => d.location = {region: "eu"});
  for (const document of documents) {
    const key = {_id: document._id, tenant: document.tenant};
    if (name === "compound") key["location.region"] = "eu";
    demo.getCollection(name).replaceOne(key, document, {upsert: true});
  }
  assert.equal(demo.getCollection(name).countDocuments({}), 4);
}
for (const document of [
  {_id: "unsharded-jacket", title: "Rain shell", description: "Waterproof hiking jacket for rainy mountain trails"},
  {_id: ObjectId("000000000000000000000013"), title: "Tent", description: "Two-person camping tent with a waterproof flysheet"},
  {_id: NumberLong("14"), title: "Sleeping bag", description: "Insulated sleeping bag for cold winter nights"},
]) {
  demo.unsharded.replaceOne({_id: document._id}, document, {upsert: true});
}
assert.equal(demo.unsharded.countDocuments({}), 3);
print("MONGOS_DATASET_READY");
