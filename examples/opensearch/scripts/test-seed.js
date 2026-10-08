const seedAssert = require("node:assert/strict");
const seedProducts = db.getSiblingDB("search_demo").products;
seedProducts.replaceOne({_id: "seed-preservation-probe"}, {
  _id: "seed-preservation-probe", title: "Keep this user document"
}, {upsert: true});
try {
  load("/scripts/seed.js");
  seedAssert.ok(seedProducts.findOne({_id: "seed-preservation-probe"}),
    "seeding must preserve documents outside the dataset");
  load("/scripts/seed.js");
  for (let i = 1; i <= 20; i++) {
    const id = `p${String(i).padStart(3, "0")}`;
    seedAssert.equal(seedProducts.countDocuments({_id: id}), 1);
  }
  print("Seed preservation and idempotency passed");
} finally {
  seedProducts.deleteOne({_id: "seed-preservation-probe"});
}
