for (const name of ["range", "hashed", "compound", "unsharded"]) {
  print(name);
  printjson(db.getSiblingDB("search_demo").getCollection(name).aggregate([
    {$vectorSearch: {path: "description", query: "waterproof hiking jacket", limit: 4}},
    {$set: {score: {$meta: "vectorSearchScore"}}}
  ]).toArray());
}
