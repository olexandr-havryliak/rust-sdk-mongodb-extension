const assert = require("node:assert/strict");
const config = {_id: process.env.RS_NAME, members: [{_id: 0, host: process.env.RS_HOST}]};
if (process.env.RS_CONFIG === "true") config.configsvr = true;
assert.equal(rs.initiate(config).ok, 1);
