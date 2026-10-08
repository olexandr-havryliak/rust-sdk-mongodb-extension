const config = {_id: process.env.RS_NAME, members: [{_id: 0, host: process.env.RS_HOST}]};
if (process.env.RS_CONFIG === "true") config.configsvr = true;
try {
  rs.status();
} catch (err) {
  if (err.code !== 94) throw err;
  const result = rs.initiate(config);
  if (result.ok !== 1) throw new Error(JSON.stringify(result));
}
