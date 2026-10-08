const assert = require("node:assert/strict");
function command(spec) {
  const result = db.adminCommand(spec);
  assert.equal(result.ok, 1, JSON.stringify(result));
  return result;
}
assert.equal(db.hello().msg, "isdbgrid");
command({balancerStop: 1});
command({addShard: "shard0/shard0:27017", name: "shard0"});
command({addShard: "shard1/shard1:27017", name: "shard1"});
assert.equal(command({listShards: 1}).shards.length, 2);
for (const [database, collection] of [["sdk_router_poc", "products"], ["sdk_router_other", "articles"]]) {
  command({enableSharding: database, primaryShard: "shard0"});
  const coll = db.getSiblingDB(database).getCollection(collection);
  assert.equal(coll.insertMany([
    {_id: -2, title: "Left two", description: "Document on shard0"},
    {_id: -1, title: "Left one", description: "Another document on shard0"},
    {_id: 1, title: "Right one", description: "Document on shard1"},
    {_id: 2, title: "Right two", description: "Another document on shard1"},
  ]).acknowledged, true);
  const namespace = `${database}.${collection}`;
  command({shardCollection: namespace, key: {_id: 1}});
  command({split: namespace, middle: {_id: 0}});
  command({moveChunk: namespace, find: {_id: 1}, to: "shard1", _waitForDelete: true});
  assert.equal(coll.countDocuments({}), 4);
}
print("ROUTER_POC_CLUSTER_READY");
