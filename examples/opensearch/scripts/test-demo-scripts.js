const demoProducts = db.getSiblingDB("search_demo").products;
const demoAssert = require("node:assert/strict");

load("/scripts/demo-upsert.js");
const original = demoProducts.findOne({ _id: "demo-shell" });
demoAssert.deepEqual(Object.keys(original).sort(), ["_id", "description", "title"]);
demoAssert.equal(original.title, "Demo Waterproof Hiking Shell");
demoAssert.ok(original.description.toLowerCase().includes("waterproof"));
load("/scripts/demo-upsert.js");
demoAssert.equal(demoProducts.countDocuments({ _id: "demo-shell" }), 1);
load("/scripts/demo-update.js");
const updated = demoProducts.findOne({ _id: "demo-shell" });
demoAssert.deepEqual(Object.keys(updated).sort(), ["_id", "description", "title"]);
demoAssert.equal(updated.title, "Demo Winter Expedition Parka");
demoAssert.notEqual(updated.description, original.description);
load("/scripts/demo-update.js");
demoAssert.equal(demoProducts.findOne({ _id: "demo-shell" }).description, updated.description);
load("/scripts/demo-upsert.js");
demoAssert.equal(demoProducts.findOne({ _id: "demo-shell" }).description, original.description);
demoProducts.deleteOne({ _id: "demo-shell" });

load("/scripts/demo-articles.js");
load("/scripts/demo-articles.js");
const demoArticles = db.getSiblingDB("catalog").articles;
for (const id of ["a001", "a002"]) {
  demoAssert.equal(demoArticles.countDocuments({ _id: id }), 1);
  demoAssert.deepEqual(Object.keys(demoArticles.findOne({ _id: id })).sort(),
    ["_id", "description", "title"]);
}
demoArticles.deleteMany({ _id: { $in: ["a001", "a002"] } });
print("Demo upsert/update script assertions passed");
