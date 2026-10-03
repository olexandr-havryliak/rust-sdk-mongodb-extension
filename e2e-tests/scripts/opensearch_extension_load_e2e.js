const dbName = "opensearch_extension_load_e2e";
const coll = db.getSiblingDB(dbName).products;

coll.drop();
coll.insertOne({
  _id: "load-smoke",
  name: "Load Smoke Product",
  description: "Waterproof shell for extension load smoke testing",
});

function assertStageLoaded(stage) {
  try {
    coll.aggregate([stage]).toArray();
  } catch (err) {
    const message = String(err);
    if (message.includes("Unrecognized pipeline stage name")) {
      throw new Error(`stage was not registered: ${message}`);
    }
    if (!message.includes("all OpenSearch endpoints failed")) {
      throw new Error(`expected OpenSearch runtime error, got: ${message}`);
    }
    return;
  }
  throw new Error("expected OpenSearch connection failure, but aggregation succeeded");
}

assertStageLoaded({
  $search: {
    path: "description",
    query: "waterproof shell",
    limit: 1,
  },
});

assertStageLoaded({
  $vectorSearch: {
    path: "description",
    query: "waterproof shell",
    limit: 1,
  },
});

print("OPENSEARCH_EXTENSION_LOAD_OK");
