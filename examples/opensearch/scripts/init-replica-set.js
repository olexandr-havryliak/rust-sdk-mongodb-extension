try {
  rs.status();
} catch (_err) {
  rs.initiate({
    _id: "rs0",
    members: [{ _id: 0, host: "mongo:27017" }],
  });
}

let ready = false;
for (let i = 0; i < 60; i++) {
  const status = rs.status();
  if (status.ok === 1 && status.myState === 1) {
    ready = true;
    break;
  }
  sleep(1000);
}

if (!ready) {
  throw new Error("replica set did not become primary");
}
