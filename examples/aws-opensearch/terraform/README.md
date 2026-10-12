# Terraform AWS Infrastructure

Create and delete infrastructure for the **Rust SDK for MongoDB Extensions**
[AWS OpenSearch example](../README.md). Run commands from the repository root.
SDK overview: [README](../../../README.md). Stage contracts:
[ARCHITECTURE.md](../../../ARCHITECTURE.md).
Only Docker is required locally; no Terraform or AWS CLI installation is needed.

## 1. Export Credentials And Settings

Use credentials with permission to manage OpenSearch domains, IAM roles/policies
and the policy attachment to your existing client user or role.

```bash
export AWS_ACCESS_KEY_ID='YOUR_ACCESS_KEY_ID'
export AWS_SECRET_ACCESS_KEY='YOUR_SECRET_ACCESS_KEY'
export AWS_SESSION_TOKEN='YOUR_SESSION_TOKEN'

export LOCAL_UID="$(id -u)"
export LOCAL_GID="$(id -g)"
```

The session token is required only for temporary credentials. For long-lived
access keys, omit its export and run `unset AWS_SESSION_TOKEN` to clear an old
token. Compose passes these standard AWS variables directly to Terraform.
Never put credentials in Terraform files or commit them to Git.

No other AWS/Terraform exports are required. Defaults:

- Region: `us-east-1`.
- Domain name: `mongodb-vector-demo`.
- Client: the IAM user or role behind the exported credentials. For temporary
  assumed-role credentials, Terraform resolves the original role ARN including
  its IAM path, not the STS session ARN. This discovery may require `iam:GetRole`.

The client receives the demo access policy; by default it is also the identity
provisioning infrastructure. Root and STS federated-user identities are not
supported as clients. Use IAM user/assumed-role credentials or an explicit client
ARN. Discovery uses the provider's
[IAM session context](https://registry.terraform.io/providers/hashicorp/aws/6.6.0/docs/data-sources/iam_session_context).

Choose a fresh domain name; this workflow does not adopt an existing domain.
Ensure Titan Text Embeddings V2 is usable in the inference region and the
account's OpenSearch service-linked role prerequisites are satisfied.
See [AWS setup](../AWS.md) for details.

Optional manual overrides:

```bash
export AWS_DEFAULT_REGION='eu-west-1'
export TF_VAR_domain_name='my-vector-demo'
export TF_VAR_client_principal_arn='arn:aws:iam::123456789012:role/development/demo-client'
```

The client override must be an existing same-account IAM user/role ARN, not an
STS session ARN. It disables client discovery and allows provisioning and demo
identities to differ.

For region selection, the Docker entrypoint uses `TF_VAR_aws_region` first, then
`AWS_DEFAULT_REGION`, then `us-east-1`. Direct Terraform runs (outside this Docker
scenario) use `aws_region`'s `us-east-1` default unless explicitly overridden.

Other optional settings (shown with defaults):

```bash
export TF_VAR_instance_type='t3.small.search'
export TF_VAR_instance_count=1
export TF_VAR_volume_size=10
# Set only when Bedrock inference uses a different region:
export TF_VAR_bedrock_region='us-east-1'
```

Alternatively use [terraform.tfvars.example](terraform.tfvars.example) as a
reference for infrastructure inputs. A local `terraform.tfvars` overrides
matching `TF_VAR_*` settings; credentials do not belong there.

## 2. Create Infrastructure

```bash
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform init -lockfile=readonly
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform validate
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform plan -out=demo.tfplan
```

Review the account, region, resource sizes and IAM policy. Apply the saved plan
(this creates billable resources without another confirmation prompt):

```bash
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform apply demo.tfplan
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform output
```

Provisioning can take many minutes. Keep state and saved plans private and backed
up. If apply is interrupted, inspect state/AWS and retry or destroy; do not delete
state to start over.

## 3. Connect And Run The Demo

Export the Terraform outputs:

```bash
export AWS_OPENSEARCH_ENDPOINT="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw opensearch_endpoint)"
export AWS_BEDROCK_ROLE_ARN="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw bedrock_role_arn)"
export AWS_CONNECT_ROLE_ARN="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw kafka_role_arn)"
export AWS_DEFAULT_REGION="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw aws_region)"
export AWS_BEDROCK_REGION="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw bedrock_region)"
export AWS_MODE=true
unset AWS_PROVISION_DOMAIN
```

Use credentials belonging to the selected client (`terraform output` shows
`client_principal_arn`). In automatic mode these are already the credentials from
step 1. If you chose a different client explicitly, re-export its credentials for
the demo, then restore provisioning credentials for Terraform operations.

Before switching to a different identity for future Terraform operations, pin the
original client so automatic discovery does not change policy attachments:

```bash
export TF_VAR_client_principal_arn="$(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm -T terraform output -raw client_principal_arn)"
```

```bash
bash examples/aws-opensearch/run-demo.sh build replica-set
bash examples/aws-opensearch/run-demo.sh up replica-set
bash examples/aws-opensearch/run-demo.sh query replica-set
```

`up` uses the existing domain, registers/deploys Bedrock Titan V2
(512-dimensional normalized vectors), installs default-model pipelines/templates
and configures Kafka connectors. Save its printed model ID as
`OPENSEARCH_MODEL_ID` for subsequent starts.

The extension reads exported credentials from the `mongo`/`mongos` container
environment. The launcher configures HTTPS and the signing region. Kafka assumes
`AWS_CONNECT_ROLE_ARN`; the extension does not. Refresh temporary credentials
and recreate containers before expiry.

For the router demo, replace `replica-set` with `mongos`. Run only one Docker
stack at a time. Use a separate domain/state directory for the other topology:
replica-set IDs and mongos document keys must not share an index. Changing the
domain name in the same state replaces the managed domain.

## 4. Delete Infrastructure

**Deletion removes the domain and all indexes, documents, models and pipelines,
plus the created roles and client policy attachment.** The existing client
identity and its other policies remain. Take any necessary backups first.

Stop Docker (use `mongos` for that topology):

```bash
AWS_MODE=true bash examples/aws-opensearch/run-demo.sh down replica-set
```

Restore provisioning credentials using the exports from step 1 if you switched
identities. Keep the original client ARN (as above), region and other Terraform
inputs/state unchanged; review and apply deletion:

```bash
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform plan -destroy -out=destroy.tfplan
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform apply destroy.tfplan
docker compose -f examples/aws-opensearch/terraform/docker-compose.yml run --rm terraform state list
```

The state list should be empty. Verify AWS deletion before discarding state.
Stopping Docker alone does not delete AWS resources or stop domain charges.

## Resources And Limits

- Public OpenSearch 2.19 domain: one `t3.small.search` node, 10 GiB gp3 by default,
  encrypted storage/node traffic, HTTPS and IAM-only access.
- Bedrock role restricted to this domain/account and Titan Text Embeddings V2.
- Kafka writer role trusted by the specified client; a domain-scoped client
  policy attached to that existing IAM user/role.
- No users/access keys, EC2, VPC, Kafka/MSK, S3 vectors or account-wide
  service-linked roles are created. MongoDB and Kafka run in Docker.

This is a non-HA PoC. More nodes alone do not configure Multi-AZ/index replicas.
The public endpoint requires signed requests; its policy denies other HTTP
principals. Privileged administrators can still change that policy.
The demo setup manages models/pipelines, not Terraform.

Instances/storage are billable until deleted; Bedrock inference costs extra.
Check regional availability/pricing before applying. Real AWS apply/destroy has
not been verified here. Local state is for single-user use; shared deployments
need a secured remote backend with locking.

## Local Checks Without AWS

```bash
bash examples/aws-opensearch/terraform/run-tests.sh
```

Runs Docker initialization, formatting, validation and mocked AWS provider tests.
No credentials or live services are needed; internet is needed for provider
downloads. Tests do not create AWS resources or prove real permissions/connectivity.
