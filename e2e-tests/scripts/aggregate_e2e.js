// Run inside container: mongosh /scripts/aggregate_e2e.js
// Covers map stage, extension YAML param (e2eExtensionParam), non-empty stage args, explain,
// downstream $match, small batch cursor, and EOF path on an empty collection.

const dbname = "rust_sdk_e2e";
const d = db.getSiblingDB(dbname);
d.dropDatabase();
d.t.insertMany([
  { _id: 1, v: "a" },
  { _id: 2, v: "b" },
]);

function fail(code, msg, extra) {
  print("FAIL:", msg, extra !== undefined ? JSON.stringify(extra) : "");
  quit(code);
}

// 1) Passthrough with empty args (still valid with expect_empty: false)
let out;
try {
  out = d.t.aggregate([{ $rustSdkE2e: {} }]).toArray();
} catch (e) {
  fail(2, "aggregate_empty_args", String(e));
}
if (out.length !== 2 || out[0]._id !== 1 || out[1]._id !== 2) {
  fail(3, "aggregate_empty_args_shape", out);
}
if (out[0].rustSdkE2eExtensionParam !== "from_extension_yaml" || out[1].rustSdkE2eExtensionParam !== "from_extension_yaml") {
  fail(31, "extension_yaml_param_missing", out);
}

// 2) Non-empty stage document (parse + logical serialize / explain paths in the SDK)
try {
  out = d.t.aggregate([{ $rustSdkE2e: { probe: 1, tag: "e2e" } }]).toArray();
} catch (e) {
  fail(4, "aggregate_nonempty_args", String(e));
}
if (out.length !== 2) {
  fail(5, "aggregate_nonempty_len", out.length);
}
if (out[0].rustSdkE2eExtensionParam !== "from_extension_yaml") {
  fail(51, "extension_yaml_param_nonempty_args", out[0]);
}

// 3) Stage followed by $match (exec set_source + get_next delegation)
try {
  out = d.t
    .aggregate([{ $rustSdkE2e: {} }, { $match: { _id: { $gte: 2 } } }])
    .toArray();
} catch (e) {
  fail(6, "aggregate_with_downstream", String(e));
}
if (out.length !== 1 || out[0]._id !== 2) {
  fail(7, "aggregate_with_downstream_shape", out);
}
if (out[0].rustSdkE2eExtensionParam !== "from_extension_yaml") {
  fail(71, "extension_yaml_param_downstream", out[0]);
}

// 4) Explain on aggregate (logical + exec explain hooks in passthrough)
let expl;
try {
  expl = d.t.explain("queryPlanner").aggregate([{ $rustSdkE2e: { explainProbe: true } }]);
} catch (e) {
  fail(8, "explain_aggregate", String(e));
}
if (typeof expl !== "object" || expl === null) {
  fail(9, "explain_not_object", expl);
}

// 5) Cursor helpers (open / iteration boundary)
let cur;
try {
  cur = d.t.aggregate([{ $rustSdkE2e: {} }], { batchSize: 1 });
  const first = cur.next();
  const second = cur.next();
  if (first == null || second == null) {
    fail(10, "cursor_batch", { first, second });
  }
  cur.close();
} catch (e) {
  fail(11, "cursor_iterate", String(e));
}

// 6) Empty collection: on_eof path still sees YAML param from initialize
d.empty.drop();
d.createCollection("empty");
let emptyOut;
try {
  emptyOut = d.empty.aggregate([{ $rustSdkE2e: {} }]).toArray();
} catch (e) {
  fail(12, "aggregate_empty_coll_eof", String(e));
}
if (emptyOut.length !== 1 || emptyOut[0].rustSdkE2eExtensionParam !== "from_extension_yaml") {
  fail(13, "extension_yaml_param_eof", emptyOut);
}

print("E2E_OK");
quit(0);
