const assert = require("node:assert/strict");
const config = {
  _id: process.env.RS_NAME,
  members: [{_id: 0, host: process.env.RS_HOST}],
};
if (process.env.RS_CONFIG === "true") config.configsvr = true;
const result = rs.initiate(config);
assert.equal(result.ok, 1, JSON.stringify(result));
