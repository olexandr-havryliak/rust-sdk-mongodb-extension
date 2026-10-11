provider "aws" {
  region = var.aws_region
  default_tags {
    tags = {
      Project   = "Rust SDK for MongoDB Extensions"
      ManagedBy = "Terraform"
      Purpose   = "OpenSearch vector demo"
    }
  }
}

data "aws_caller_identity" "current" {}
data "aws_partition" "current" {}

data "aws_iam_session_context" "client" {
  count = var.client_principal_arn == null ? 1 : 0
  arn   = data.aws_caller_identity.current.arn
}

locals {
  client_principal_arn = var.client_principal_arn != null ? var.client_principal_arn : data.aws_iam_session_context.client[0].issuer_arn
  domain_arn           = "arn:aws:es:${var.aws_region}:${data.aws_caller_identity.current.account_id}:domain/${var.domain_name}"
  bedrock_region       = coalesce(var.bedrock_region, var.aws_region)
  client_is_user       = can(regex("^arn:aws:iam::[0-9]{12}:user/", local.client_principal_arn))
  client_is_role       = can(regex("^arn:aws:iam::[0-9]{12}:role/", local.client_principal_arn))
  client_name          = basename(local.client_principal_arn)
  client_account       = try(split(":", local.client_principal_arn)[4], "")
  domain_resources     = "${local.domain_arn}/*"
  http_actions         = ["es:ESHttpGet", "es:ESHttpHead", "es:ESHttpPost", "es:ESHttpPut", "es:ESHttpDelete"]
}

resource "aws_iam_role" "bedrock" {
  name = "${var.domain_name}-bedrock"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Action    = "sts:AssumeRole"
      Principal = { Service = "opensearchservice.amazonaws.com" }
      Condition = {
        StringEquals = { "aws:SourceAccount" = data.aws_caller_identity.current.account_id }
        ArnEquals    = { "aws:SourceArn" = local.domain_arn }
      }
    }]
  })
}

resource "aws_iam_role_policy" "bedrock" {
  name = "invoke-titan-v2"
  role = aws_iam_role.bedrock.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["bedrock:InvokeModel"]
      Resource = "arn:aws:bedrock:${local.bedrock_region}::foundation-model/amazon.titan-embed-text-v2:0"
    }]
  })
}

resource "aws_iam_role" "kafka" {
  name = "${var.domain_name}-kafka"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Action    = "sts:AssumeRole"
      Principal = { AWS = local.client_principal_arn }
    }]
  })
}

resource "aws_iam_role_policy" "kafka" {
  name = "write-demo-domain"
  role = aws_iam_role.kafka.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = local.http_actions
      Resource = local.domain_resources
    }]
  })
}

resource "aws_iam_policy" "client" {
  name        = "${var.domain_name}-client"
  description = "Configure the demo domain, sign extension queries and assume the Kafka writer role."
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = local.http_actions, Resource = local.domain_resources },
      { Effect = "Allow", Action = ["iam:PassRole"], Resource = aws_iam_role.bedrock.arn },
      { Effect = "Allow", Action = ["sts:AssumeRole"], Resource = aws_iam_role.kafka.arn },
    ]
  })
}

resource "aws_iam_user_policy_attachment" "client" {
  count      = local.client_is_user ? 1 : 0
  user       = local.client_name
  policy_arn = aws_iam_policy.client.arn
}

resource "aws_iam_role_policy_attachment" "client" {
  count      = local.client_is_role ? 1 : 0
  role       = local.client_name
  policy_arn = aws_iam_policy.client.arn
}

resource "aws_opensearch_domain" "demo" {
  domain_name    = var.domain_name
  engine_version = "OpenSearch_2.19"

  cluster_config {
    instance_type                 = var.instance_type
    instance_count                = var.instance_count
    dedicated_master_enabled      = false
    zone_awareness_enabled        = false
    multi_az_with_standby_enabled = false
  }
  ebs_options {
    ebs_enabled = true
    volume_type = "gp3"
    volume_size = var.volume_size
  }
  encrypt_at_rest { enabled = true }
  node_to_node_encryption { enabled = true }
  domain_endpoint_options {
    enforce_https       = true
    tls_security_policy = "Policy-Min-TLS-1-2-2019-07"
  }
  # T3 instances do not support Auto-Tune. The PoC uses IAM-only access control.
  auto_tune_options { desired_state = "DISABLED" }
  advanced_security_options { enabled = false }

  access_policies = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect    = "Allow"
        Principal = { AWS = [local.client_principal_arn, aws_iam_role.kafka.arn] }
        Action    = "es:ESHttp*"
        Resource  = local.domain_resources
      },
      {
        Effect    = "Deny"
        Principal = "*"
        Action    = "es:ESHttp*"
        Resource  = local.domain_resources
        Condition = {
          ArnNotEquals = { "aws:PrincipalArn" = [local.client_principal_arn, aws_iam_role.kafka.arn] }
        }
      },
    ]
  })

  lifecycle {
    precondition {
      condition     = data.aws_partition.current.partition == "aws" && local.client_account == data.aws_caller_identity.current.account_id && (local.client_is_user || local.client_is_role)
      error_message = "Use a same-account IAM user/role in the commercial AWS partition. Root and federated-user credentials cannot be demo clients; set client_principal_arn explicitly or use IAM user/assumed-role credentials."
    }
  }
}
