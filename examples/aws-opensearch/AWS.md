# Real AWS Setup

Local commands: [README.md](README.md). Data flow: [ARCHITECTURE.md](ARCHITECTURE.md).
This mode uses provisioned Amazon OpenSearch Service, not Serverless.
OpenSearch's remote ML connector calls on-demand Bedrock Titan Text Embeddings V2
for both document and query embeddings.

For reproducible domain/IAM creation and deletion, use the
[Terraform HOWTO](terraform/README.md). It creates a public HTTPS domain and
attaches the required demo policy to an existing same-account client principal,
without creating access keys. Use its outputs below and leave
`AWS_PROVISION_DOMAIN` unset: do not mix Terraform ownership with Python domain
creation. The remaining sections describe manual setup and runtime configuration.

**Status:** configuration and signing are tested locally; real AWS resources,
IAM policies and Bedrock calls have not been verified. A real-account smoke test
is required before production use. Domains and Bedrock calls incur charges.
One small instance is a PoC starting point, not a guarantee of lowest price,
capacity or availability in every region.

## Prerequisites

- An OpenSearch 2.19 domain reachable from Docker, or permission to create one
  explicitly. Private VPC domains need suitable network access.
- Bedrock Titan V2 access/availability in the selected inference region.
- An IAM model role trusted by OpenSearch and allowed to invoke Titan V2.
- Credentials allowed to configure/search/write the domain and pass that role.
  Domain policies must permit the relevant client principals.
- With fine-grained access control, map the setup principal to `ml_full_access`
  and grant the relevant pipeline/index permissions.
- Docker Compose supporting `!reset` overrides.

Access-key variables cannot create trust relationships or bypass domain
policies. Prepare IAM first; see
[AWS ML connector prerequisites](https://docs.aws.amazon.com/opensearch-service/latest/developerguide/ml-amazon-connector.html).

## IAM Model Role

Example trust policy, substituting your account, domain and region:

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Principal": {"Service": "opensearchservice.amazonaws.com"},
    "Action": "sts:AssumeRole",
    "Condition": {
      "StringEquals": {"aws:SourceAccount": "123456789012"},
      "ArnLike": {"aws:SourceArn": "arn:aws:es:us-east-1:123456789012:domain/mongodb-vector-demo"}
    }
  }]
}
```

Attach permission to invoke the model in the inference region:

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": "bedrock:InvokeModel",
    "Resource": "arn:aws:bedrock:us-east-1::foundation-model/amazon.titan-embed-text-v2:0"
  }]
}
```

The setup caller needs `iam:PassRole` on this model role and `es:ESHttpGet`,
`es:ESHttpPost`, `es:ESHttpPut` on the domain's `/*` ARN. Runtime callers need
appropriate search or bulk/index/delete HTTP permissions. Optional domain
creation also requires OpenSearch control-plane permissions and any required
account service-linked role. The script does not create IAM roles/policies.

## Existing Domain

From the repository root, set standard environment variables. Do not put real
credentials in tracked files:

```bash
export AWS_ACCESS_KEY_ID='your-access-key-id'
export AWS_SECRET_ACCESS_KEY='your-secret-access-key'
export AWS_DEFAULT_REGION='us-east-1'
export AWS_OPENSEARCH_ENDPOINT='https://search-your-domain.us-east-1.es.amazonaws.com'
export AWS_BEDROCK_ROLE_ARN='arn:aws:iam::123456789012:role/opensearch-bedrock'
export AWS_MODE=true
bash examples/aws-opensearch/run-demo.sh build replica-set
bash examples/aws-opensearch/run-demo.sh up replica-set
bash examples/aws-opensearch/run-demo.sh query replica-set
```

For temporary credentials, also set `AWS_SESSION_TOKEN` and
`AWS_CONNECT_ROLE_ARN`. Aiven 4.1's basic provider cannot use session tokens;
its STS provider uses the AWS default credential chain to assume a writer role:

```bash
export AWS_SESSION_TOKEN='your-session-token'
export AWS_CONNECT_ROLE_ARN='arn:aws:iam::123456789012:role/kafka-opensearch-writer'
```

The writer role must trust your caller and permit domain writes/deletes. Your
caller must be allowed to assume it, and the domain policy must permit both
runtime principals. Extension credentials are environment snapshots, not an
automatic refresh implementation: restart MongoDB with fresh credentials before
expiry. Kafka refreshes assumed-role sessions, but its original environment
credentials must remain valid too.

Set `AWS_BEDROCK_REGION` only for inference in another region. The model is
`amazon.titan-embed-text-v2:0`, with normalization and 512 float dimensions.
OpenSearch uses the built-in Bedrock embedding pre/post-processors from the
[Titan blueprint](https://github.com/opensearch-project/ml-commons/blob/2.19.6.0/docs/remote_inference_blueprints/bedrock_connector_titan_embedding_blueprint.md).

`aws.yml` excludes Floci and the local embedding service. Its configuration job
runs `scripts/setup_aws.py`, verifies inference, installs ingest/search pipelines
and the `mongodb.*` index template, then writes the extension endpoint. Connect
uses SigV4 service `es` and a bare domain hostname, required by Aiven's transport.
The extension uses verified HTTPS and SigV4; AWS mode does not enable HTTP or
redirects.

The setup prints the model ID. Set `OPENSEARCH_MODEL_ID` on subsequent starts
to reuse that deployed model rather than creating another connector/model.
Do not use unrelated existing `mongodb.*` indexes: setup updates shared
pipelines/templates, and existing mappings do not change dimension automatically.

## Explicit Domain Creation

After building the images, leave the endpoint unset and opt into billable
creation explicitly:

```bash
unset AWS_OPENSEARCH_ENDPOINT
export AWS_PROVISION_DOMAIN=true
export AWS_DOMAIN_NAME='mongodb-vector-demo'
export AWS_DOMAIN_PRINCIPAL_ARN='arn:aws:iam::123456789012:user/demo-client'
export AWS_MODE=true
bash examples/aws-opensearch/run-demo.sh up replica-set
```

The request selects OpenSearch 2.19, one `t3.small.search`, 10 GB gp3, encryption
at rest/between nodes and an HTTPS-only endpoint. Its domain policy permits
only the supplied IAM principal. Additional runtime principals, such as the
Kafka assumed role, require another domain-policy grant. Instance support
varies by region; review [AWS instance requirements](https://docs.aws.amazon.com/opensearch-service/latest/developerguide/supported-instance-types.html)
and pricing before creation. This is not HA.

If the domain name already exists, the script uses it without changing its
control-plane settings. A timeout or later setup failure does not roll back
creation: check the AWS console because the domain may remain billable.

## Mongos And Cleanup

Replace `replica-set` with `mongos` for the other topology. Stop the first stack
before starting it. Use a separate fresh AWS domain/index set: raw replica-set
IDs and mongos full-document-key IDs are incompatible in the same index. Local
and AWS vector dimensions differ too.

```bash
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh down replica-set
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh build mongos
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh up mongos
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh query mongos
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh down mongos
```

`down` deletes only Docker containers and their local volumes, never AWS domains,
models or indexes. Delete unwanted domains explicitly using the AWS console or
control API to stop domain charges. Record the domain name/model ID outside the
disposable state volume. Integration tests are limited to Floci, not real AWS.

S3 Vectors is deferred. This setup creates ordinary OpenSearch k-NN indexes.
