const dbName = "search_demo";
const collName = "products";
const products = db.getSiblingDB(dbName).getCollection(collName);
const datasetPath = "/datasets/outdoor-products.json";
const fs = require("fs");
const productsDataset = JSON.parse(fs.readFileSync(datasetPath, "utf8"));

products.drop();
products.insertMany(productsDataset);

printjson({ seeded: products.countDocuments(), datasetPath });
