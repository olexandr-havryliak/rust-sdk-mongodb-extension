# AWS OpenSearch Example

MongoDB -> Kafka -> OpenSearch synchronization and text-based `$vectorSearch`
using the **Rust SDK for MongoDB Extensions**. Both replica-set and mongos
extension libraries are built from this example, using the shared
[OpenSearch core](../../opensearch-extension-core/README.md) with HTTPS and AWS SigV4 transport.

## Local HOWTO

Run commands from the repository root. Only Docker and Docker Compose are
required; MongoDB clients, Rust, Python and models run in containers. The first
start downloads the public embedding model and needs internet access.
Keep `AWS_MODE` unset (or `false`) for the commands in this section. The local
launcher substitutes test credentials even when real AWS keys exist in your shell.

Build and start the replica-set example:

```bash
bash examples/aws-opensearch/run-demo.sh build replica-set
bash examples/aws-opensearch/run-demo.sh up replica-set
```

Startup seeds 20 outdoor products from this example\'s
[`outdoor-products.json`](datasets/outdoor-products.json). Each product has `_id`, `title` and
`description`. Kafka forwards only `description` for indexing; `title` remains
in MongoDB. Initial copying and subsequent change streams are automatic.

Insert or upsert a document, then read it from MongoDB:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml exec -T mongo mongosh --quiet --eval 'db.getSiblingDB("search_demo").products.replaceOne({_id:"demo-shell"},{_id:"demo-shell",title:"Rain shell",description:"Waterproof hiking jacket for rainy mountain trails"},{upsert:true}); printjson(db.getSiblingDB("search_demo").products.findOne({_id:"demo-shell"}));'
```

Floci publishes its first domain on port 9400. Check the actual port before
using these HTTP commands; substitute that port if 9400 was already occupied:

```bash
docker port floci-aws-opensearch-mongodb-vector-demo 9200/tcp
curl -fsS 'http://localhost:9400/mongodb.search_demo.products/_count'
curl -fsS 'http://localhost:9400/mongodb.search_demo.products/_doc/demo-shell?filter_path=_id,_source'
```

Allow a few seconds for synchronization. `_source` contains a
`description_embedding` array of 384 numbers, not `title` or `description`.
Compare it with the response after this update:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml exec -T mongo mongosh --quiet --eval 'db.getSiblingDB("search_demo").products.updateOne({_id:"demo-shell"},{$set:{title:"Winter sleeping bag",description:"Insulated sleeping bag for cold winter nights"}}); printjson(db.getSiblingDB("search_demo").products.findOne({_id:"demo-shell"}));'
curl -fsS 'http://localhost:9400/mongodb.search_demo.products/_doc/demo-shell?filter_path=_id,_source'
```

The OpenSearch ID stays the same, but the vector changes. Delete the document;
the OpenSearch GET eventually returns HTTP 404:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml exec -T mongo mongosh --quiet --eval 'db.getSiblingDB("search_demo").products.deleteOne({_id:"demo-shell"});'
curl -i 'http://localhost:9400/mongodb.search_demo.products/_doc/demo-shell'
```

Search directly in OpenSearch, returning only IDs and scores. Its search
pipeline supplies the model ID automatically:

```bash
curl -fsS -X POST 'http://localhost:9400/mongodb.search_demo.products/_search?allow_partial_search_results=false&filter_path=hits.hits._id,hits.hits._score' \
  -H 'Content-Type: application/json' \
  -d '{"size":3,"_source":false,"query":{"neural":{"description_embedding":{"query_text":"waterproof hiking jacket","k":3}}}}'
```

Run the corresponding MongoDB aggregation:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml exec -T mongo mongosh --quiet --eval 'printjson(db.getSiblingDB("search_demo").products.aggregate([{$vectorSearch:{path:"description",query:"waterproof hiking jacket",limit:3}},{$set:{score:{$meta:"vectorSearchScore"}}}]).toArray());'
```

MongoDB returns complete current documents, including `title`, plus `score`.
The stage is this example's text-query interface, not Atlas's standard
`queryVector` interface. `$search` is not implemented.

Stop and remove the domain, containers and all demo volumes:

```bash
bash examples/aws-opensearch/run-demo.sh down replica-set
```

### Mongos

Start only after removing the replica-set stack:

```bash
bash examples/aws-opensearch/run-demo.sh build mongos
bash examples/aws-opensearch/run-demo.sh up mongos
bash examples/aws-opensearch/run-demo.sh query mongos
```

This creates one mongos, one single-node CSRS and two single-node shards.
The extension is loaded only in mongos. `search_demo.products` uses
`{_id:"hashed"}` and the same 20-product dataset. Additional collections exercise
range, hashed and compound shard keys, duplicate IDs across shards, BSON key
types and an unsharded collection. Dataset shapes and shard keys are defined in
[`scripts/mongos/seed.js`](scripts/mongos/seed.js) and
[`scripts/mongos/init-cluster.js`](scripts/mongos/init-cluster.js).

Insert/update a product and query through mongos:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/mongos.yml exec -T mongos mongosh --quiet --eval 'db.getSiblingDB("search_demo").products.replaceOne({_id:"demo-shell"},{_id:"demo-shell",title:"Rain shell",description:"Waterproof hiking jacket for rainy mountain trails"},{upsert:true});'
curl -fsS -X POST 'http://localhost:9400/mongodb.search_demo.products/_search?filter_path=hits.hits._id,hits.hits._source' \
  -H 'Content-Type: application/json' \
  -d '{"query":{"ids":{"values":["{\"_id\": \"demo-shell\"}"]}}}'
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/mongos.yml exec -T mongos mongosh --quiet --eval 'db.getSiblingDB("search_demo").products.updateOne({_id:"demo-shell"},{$set:{description:"Insulated sleeping bag for cold winter nights"}});'
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/mongos.yml exec -T mongos mongosh --quiet --eval 'printjson(db.getSiblingDB("search_demo").products.aggregate([{$vectorSearch:{path:"description",query:"waterproof hiking jacket",limit:3}},{$set:{score:{$meta:"vectorSearchScore"}}}]).toArray());'
bash examples/aws-opensearch/run-demo.sh down mongos
```

Mongos OpenSearch IDs contain the connector's canonical Extended JSON document
key, not a raw string ID. `_source.__mongodb` retains lookup metadata alongside
vectors. Do not strip or reformat the stored identity. Allow synchronization
and index refresh before comparing search results.

### Verification

Unit/configuration tests do not need a service stack. Integration runs build,
start and clean up automatically, including on test failure:

```bash
bash examples/aws-opensearch/run-demo.sh unit replica-set
bash examples/aws-opensearch/run-demo.sh down replica-set
bash examples/aws-opensearch/run-demo.sh test replica-set
bash examples/aws-opensearch/run-demo.sh test mongos
```

Tests verify local inference, initial copy, insert/update/replace/delete/reinsert,
vector-only storage, default query embeddings and matching MongoDB/OpenSearch
IDs, scores and full documents. They do not validate AWS IAM, TLS enforcement
or Bedrock availability. General SDK checks: [TESTING.md](../../TESTING.md).

## Details

- [Repository README](../../README.md): why the SDK exists and how this example fits
- [SDK architecture](../../ARCHITECTURE.md): host lifecycle, stage kinds, and the ABI contract
- [Example index](../README.md)
- [Architecture and automatic setup](ARCHITECTURE.md)
- [Real AWS domain and Bedrock setup](AWS.md)
- [Terraform creation/deletion and Docker-only HOWTO](terraform/README.md)
- [Native Kafka synchronization and connector lifecycle](SYNC.md)

Local containers disable TLS/auth and extension signature validation for
unsigned development libraries. Floci mounts the Docker socket and controls
host Docker containers. Neither choice is production guidance.
