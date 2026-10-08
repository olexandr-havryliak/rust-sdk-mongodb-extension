const dbName = "search_demo";
const collName = "products";
const products = db.getSiblingDB(dbName).getCollection(collName);
const datasetPath = "/datasets/outdoor-products.json";
const fs = require("fs");
const productsDataset = JSON.parse(fs.readFileSync(datasetPath, "utf8"));

products.bulkWrite(productsDataset.map((document) => ({
  replaceOne: {
    filter: { _id: document._id },
    replacement: document,
    upsert: true,
  },
})));

printjson({ seeded: products.countDocuments(), datasetPath });
