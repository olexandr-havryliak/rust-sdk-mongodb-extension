{
  const articles = db.getSiblingDB("catalog").articles;
  printjson(articles.bulkWrite([
    { replaceOne: {
      filter: { _id: "a001" },
      replacement: {
        _id: "a001", title: "Mountain camping",
        description: "A guide to warm sleeping bags and insulated pads for cold mountain camps.",
      },
      upsert: true,
    } },
    { replaceOne: {
      filter: { _id: "a002" },
      replacement: {
        _id: "a002", title: "Rainy trail essentials",
        description: "Waterproof jackets and rain covers keep hikers and backpacks dry on wet trails.",
      },
      upsert: true,
    } },
  ]));
}
