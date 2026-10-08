{
  const products = db.getSiblingDB("search_demo").products;
  const result = products.replaceOne(
    { _id: "demo-shell" },
    {
      _id: "demo-shell",
      title: "Demo Waterproof Hiking Shell",
      description: "Waterproof hiking shell with taped seams and a breathable hood for rainy mountain trails.",
    },
    { upsert: true }
  );
  printjson(result);
  printjson(products.findOne({ _id: "demo-shell" }));
}
