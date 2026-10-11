db.getSiblingDB("search_demo").range.updateOne(
  {_id: "shared", tenant: -1},
  {$set: {title: "Updated rain shell", description: "A breathable waterproof jacket for long mountain hikes"}}
);
