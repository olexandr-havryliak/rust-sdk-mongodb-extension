{
  const products = db.getSiblingDB("search_demo").products;
  const result = products.replaceOne(
    { _id: "demo-shell" },
    {
      _id: "demo-shell",
      name: "Demo Waterproof Hiking Shell",
      description: "Waterproof hiking shell with taped seams and a breathable hood for rainy mountain trails.",
      category: "outerwear",
      price: 159,
      inStock: true,
      updatedAt: "2026-10-03T10:00:00Z",
      internalNotes: "MongoDB-only demo marker",
    },
    { upsert: true }
  );
  printjson(result);
  printjson(products.findOne({ _id: "demo-shell" }));
}
