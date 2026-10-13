const assert = require("node:assert/strict");
assert.equal(sh.addShard("shard0/shard0:27017").ok, 1);
assert.equal(sh.addShard("shard1/shard1:27017").ok, 1);
assert.equal(sh.enableSharding("rust_sdk_performance").ok, 1);
assert.equal(db.adminCommand({shardCollection: "rust_sdk_performance.input", key: {_id: "hashed"}, numInitialChunks: 2}).ok, 1);
