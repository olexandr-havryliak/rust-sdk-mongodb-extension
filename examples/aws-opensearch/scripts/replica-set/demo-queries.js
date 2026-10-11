const dbName = "search_demo";
const collName = "products";
const products = db.getSiblingDB(dbName).getCollection(collName);

function printSection(title, cursor) {
  print(`\n## ${title}`);
  cursor.forEach((doc) => {
    printjson({
      _id: doc._id,
      title: doc.title,
      description: doc.description,
      score: doc.score,
    });
  });
}

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
        title: 1,
        description: 1,
        score: { $meta: "vectorSearchScore" },
      },
    },
  ])
);
