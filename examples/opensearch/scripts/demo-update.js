{
  const products = db.getSiblingDB("search_demo").products;
  const result = products.updateOne(
    { _id: "demo-shell" },
    {
      $set: {
        description: "Insulated winter expedition parka with a warm hood for freezing weather and snowy mountain camps.",
        title: "Demo Winter Expedition Parka",
      },
    }
  );
  require("node:assert/strict").equal(result.matchedCount, 1, "Run demo-upsert.js first");
  printjson(result);
  printjson(products.findOne({ _id: "demo-shell" }));
}
