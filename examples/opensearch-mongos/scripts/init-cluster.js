const assert = require("node:assert/strict");
function command(spec) {
  const result = db.adminCommand(spec);
  assert.equal(result.ok, 1, JSON.stringify(result));
  return result;
}
assert.equal(db.hello().msg, "isdbgrid");
command({balancerStop: 1});
for (const name of ["shard0", "shard1"]) {
  if (!command({listShards: 1}).shards.some(s => s._id === name)) {
    command({addShard: `${name}/${name}:27017`, name});
  }
}
command({enableSharding: "search_demo", primaryShard: "shard0"});
for (const [name, key] of [["range", {tenant: 1}], ["hashed", {tenant: "hashed"}],
                         ["compound", {"location.region": 1, tenant: 1}]]) {
  const catalog = db.getSiblingDB("config");
  const existing = catalog.collections.findOne({_id: `search_demo.${name}`, dropped: {$ne: true}});
  if (existing && name !== "hashed") continue;
  if (!existing) command({shardCollection: `search_demo.${name}`, key, ...(name === "hashed" ? {numInitialChunks: 2} : {})});
  if (name === "hashed") {
    const uuid = catalog.collections.findOne({_id: "search_demo.hashed"}).uuid;
    // Both demo tenants hash to positive values; the default split at zero is insufficient.
    const middle = convertShardKeyToHashed(1);
    if (!catalog.chunks.findOne({uuid, "min.tenant": middle})) {
      command({split: "search_demo.hashed", middle: {tenant: middle}});
    }
    for (const [tenant, to] of [[-1, "shard0"], [1, "shard1"]]) {
      const hashed = convertShardKeyToHashed(tenant);
      const chunk = catalog.chunks.findOne({uuid, $expr: {$and: [
        {$lte: ["$min.tenant", hashed]}, {$gt: ["$max.tenant", hashed]}
      ]}});
      if (chunk.shard !== to) command({moveChunk: "search_demo.hashed", find: {tenant}, to, _waitForDelete: true});
    }
  }
  if (name !== "hashed") {
    const middle = name === "compound" ? {"location.region": "eu", tenant: 0} : {tenant: 0};
    command({split: `search_demo.${name}`, middle});
    command({moveChunk: `search_demo.${name}`, find: name === "compound" ? {"location.region": "eu", tenant: 1} : {tenant: 1},
      to: "shard1", _waitForDelete: true});
  }
}
print("SHARDED_CLUSTER_READY");
