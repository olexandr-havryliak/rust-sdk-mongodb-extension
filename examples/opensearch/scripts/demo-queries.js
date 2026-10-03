const dbName = "search_demo";
const collName = "products";
const products = db.getSiblingDB(dbName).getCollection(collName);

function printSection(title, cursor) {
  print(`\n## ${title}`);
  cursor.forEach((doc) => {
    printjson({
      _id: doc._id,
      name: doc.name,
      category: doc.category,
      price: doc.price,
      inStock: doc.inStock,
      score: doc.score,
    });
  });
}

printSection(
  "Text search: waterproof hiking shell",
  products.aggregate([
    {
      $search: {
        path: "description",
        query: "waterproof hiking shell",
        limit: 5,
      },
    },
    {
      $project: {
        _id: 1,
        name: 1,
        category: 1,
        price: 1,
        inStock: 1,
        score: { $meta: "searchScore" },
      },
    },
  ])
);

printSection(
  "Vector search: warm sleep system for cold backpacking",
  products.aggregate([
    {
      $vectorSearch: {
        path: "description",
        query: "warm sleep system for cold backpacking",
        limit: 5,
      },
    },
    {
      $project: {
        _id: 1,
        name: 1,
        category: 1,
        price: 1,
        inStock: 1,
        score: { $meta: "vectorSearchScore" },
      },
    },
  ])
);
