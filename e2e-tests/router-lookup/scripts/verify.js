const assert = require("node:assert/strict");
function assertDocuments(actual, expected, message) {
  // mongosh returns BSON objects from a different JS realm; compare normalized values.
  assert.deepEqual(JSON.parse(EJSON.stringify(actual)), JSON.parse(EJSON.stringify(expected)), message);
}
const candidates = [
  {_id: 2, score: 0.95}, {_id: -2, score: 0.9}, {_id: 1, score: 0.8},
  {_id: -1, score: 0.7}, {_id: 404, score: 0.6}, {_id: 2, score: 0.5},
];
for (const [database, collection] of [["sdk_router_poc", "products"], ["sdk_router_other", "articles"]]) {
  const coll = db.getSiblingDB(database).getCollection(collection);
  const expected = candidates.filter(c => c._id !== 404).map(c => ({...coll.findOne({_id: c._id}), score: c.score}));
  const stage = {$routerLookupPoc: {candidates}};
  const explain = coll.explain().aggregate([stage]);
  assert.equal(explain.mergeType, "router");
  assert.ok(explain.splitPipeline.shardsPart.some(stage => stage.$match), "source-only plan must suppress shard input");
  for (const shard of Object.values(explain.shards)) {
    assert.ok(!JSON.stringify(shard.queryPlanner.winningPlan).includes("COLLSCAN"), "candidate generation must not scan shards");
  }
  print(`DPL_EXPLAIN ${JSON.stringify(explain)}`);
  const actual = coll.aggregate([stage], {maxTimeMS: 15000}).toArray();
  print(`DPL_ACTUAL ${JSON.stringify(actual)}`);
  // Positive control: ordinary lookup can fetch from both shards on this router.
  const suffix = [
    {$lookup: {from: collection, localField: "_id", foreignField: "_id", as: "__pocDocument"}},
    {$unwind: "$__pocDocument"},
    {$replaceWith: {$mergeObjects: ["$__pocDocument", {score: "$score"}]}},
  ];
  assertDocuments(coll.aggregate([stage, ...suffix], {maxTimeMS: 15000}).toArray(), expected);
  print(`NATIVE_LOOKUP_CONTROL_OK ${database}.${collection}`);
  assertDocuments(actual, expected, "DPL must automatically add lookup, preserve rank/score, and omit missing IDs");
  assertDocuments(coll.aggregate([{$routerLookupPoc: {candidates: []}}], {maxTimeMS: 15000}).toArray(), []);
  coll.updateOne({_id: 2}, {$set: {title: "Fresh title"}});
  const fresh = coll.aggregate([{$routerLookupPoc: {candidates: [{_id: 2, score: 0.99}]}}], {maxTimeMS: 15000}).toArray();
  assert.equal(fresh[0].title, "Fresh title");
  assert.equal(fresh[0].score, 0.99);
}
print("ROUTER_LOOKUP_POC_OK");
