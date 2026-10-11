mock_provider "aws" {
  mock_data "aws_caller_identity" {
    defaults = { account_id = "123456789012" }
  }
  mock_data "aws_partition" {
    defaults = { partition = "aws" }
  }
}

override_resource {
  target = aws_iam_role.bedrock
  values = { arn = "arn:aws:iam::123456789012:role/mongodb-vector-demo-bedrock" }
}

override_resource {
  target = aws_iam_role.kafka
  values = { arn = "arn:aws:iam::123456789012:role/mongodb-vector-demo-kafka" }
}

override_resource {
  target = aws_opensearch_domain.demo
  values = { endpoint = "search-demo.us-east-1.es.amazonaws.com" }
}

override_resource {
  target = aws_iam_policy.client
  values = { arn = "arn:aws:iam::123456789012:policy/mongodb-vector-demo-client" }
}

variables {
  client_principal_arn = "arn:aws:iam::123456789012:user/demo-client"
}

run "encrypted_public_poc" {
  command = apply
  assert {
    condition     = aws_opensearch_domain.demo.engine_version == "OpenSearch_2.19" && aws_opensearch_domain.demo.cluster_config[0].instance_count == 1
    error_message = "Use the current demo engine with a single-node default."
  }
  assert {
    condition     = aws_opensearch_domain.demo.encrypt_at_rest[0].enabled && aws_opensearch_domain.demo.node_to_node_encryption[0].enabled && aws_opensearch_domain.demo.domain_endpoint_options[0].enforce_https
    error_message = "Storage, node traffic and the public endpoint must be encrypted."
  }
  assert {
    condition     = aws_opensearch_domain.demo.domain_endpoint_options[0].tls_security_policy == "Policy-Min-TLS-1-2-2019-07" && length(aws_opensearch_domain.demo.vpc_options) == 0
    error_message = "This PoC must use a public TLS 1.2+ endpoint, not an unreachable VPC domain."
  }
  assert {
    condition     = toset(jsondecode(aws_opensearch_domain.demo.access_policies).Statement[0].Principal.AWS) == toset([var.client_principal_arn, aws_iam_role.kafka.arn])
    error_message = "Only the explicit client and Kafka writer may access the domain."
  }
  assert {
    condition     = jsondecode(aws_opensearch_domain.demo.access_policies).Statement[0].Resource == "arn:aws:es:us-east-1:123456789012:domain/mongodb-vector-demo/*"
    error_message = "Domain access must not grant access to unrelated AWS resources."
  }
}

run "bedrock_and_writer_roles" {
  command = apply
  assert {
    condition     = jsondecode(aws_iam_role.bedrock.assume_role_policy).Statement[0].Principal.Service == "opensearchservice.amazonaws.com" && jsondecode(aws_iam_role.bedrock.assume_role_policy).Statement[0].Condition.StringEquals["aws:SourceAccount"] == "123456789012" && jsondecode(aws_iam_role.bedrock.assume_role_policy).Statement[0].Condition.ArnEquals["aws:SourceArn"] == "arn:aws:es:us-east-1:123456789012:domain/mongodb-vector-demo"
    error_message = "Bedrock trust must be limited to this OpenSearch domain/account."
  }
  assert {
    condition     = jsondecode(aws_iam_role_policy.bedrock.policy).Statement[0].Action == ["bedrock:InvokeModel"] && jsondecode(aws_iam_role_policy.bedrock.policy).Statement[0].Resource == "arn:aws:bedrock:us-east-1::foundation-model/amazon.titan-embed-text-v2:0"
    error_message = "Permit only invocation of the selected Titan V2 model."
  }
  assert {
    condition     = jsondecode(aws_iam_role.kafka.assume_role_policy).Statement[0].Principal.AWS == var.client_principal_arn && jsondecode(aws_iam_role_policy.kafka.policy).Statement[0].Resource == "arn:aws:es:us-east-1:123456789012:domain/mongodb-vector-demo/*"
    error_message = "Kafka must trust the explicit caller and write only to this domain."
  }
  assert {
    condition     = jsondecode(aws_iam_policy.client.policy).Statement[1].Resource == aws_iam_role.bedrock.arn && jsondecode(aws_iam_policy.client.policy).Statement[2].Resource == aws_iam_role.kafka.arn
    error_message = "PassRole and AssumeRole must target only the created roles."
  }
  assert {
    condition     = length(aws_iam_user_policy_attachment.client) == 1 && length(aws_iam_role_policy_attachment.client) == 0 && aws_iam_user_policy_attachment.client[0].user == "demo-client"
    error_message = "Attach the additional policy to an existing user without creating access keys."
  }
  assert {
    condition     = output.opensearch_endpoint == "https://search-demo.us-east-1.es.amazonaws.com" && output.bedrock_role_arn == aws_iam_role.bedrock.arn && output.kafka_role_arn == aws_iam_role.kafka.arn && output.aws_region == "us-east-1"
    error_message = "Outputs must connect the existing Docker example without credentials in state."
  }
}

run "other_principals_explicitly_denied" {
  command = apply
  assert {
    condition     = try(jsondecode(aws_opensearch_domain.demo.access_policies).Statement[1].Effect, "") == "Deny" && try(jsondecode(aws_opensearch_domain.demo.access_policies).Statement[1].Principal, "") == "*"
    error_message = "Other callers must be explicitly denied even if they have an identity-based ESHttp allow."
  }
  assert {
    condition     = try(toset(jsondecode(aws_opensearch_domain.demo.access_policies).Statement[1].Condition.ArnNotEquals["aws:PrincipalArn"]), toset([])) == toset([var.client_principal_arn, aws_iam_role.kafka.arn])
    error_message = "The deny exception must use stable IAM principal ARNs, including the Kafka assumed role."
  }
}

run "role_path_and_custom_region" {
  command = apply
  variables {
    client_principal_arn = "arn:aws:iam::123456789012:role/development/demo-client"
    aws_region           = "eu-west-1"
    bedrock_region       = "eu-central-1"
    domain_name          = "another-demo"
    instance_type        = "t3.medium.search"
    instance_count       = 2
    volume_size          = 20
  }
  assert {
    condition     = length(aws_iam_user_policy_attachment.client) == 0 && aws_iam_role_policy_attachment.client[0].role == "demo-client"
    error_message = "An IAM role ARN with a path must attach to the role name."
  }
  assert {
    condition     = aws_opensearch_domain.demo.cluster_config[0].instance_count == 2 && aws_opensearch_domain.demo.cluster_config[0].instance_type == "t3.medium.search" && aws_opensearch_domain.demo.ebs_options[0].volume_size == 20
    error_message = "Instance and storage parameters must reach the domain."
  }
  assert {
    condition     = jsondecode(aws_iam_role_policy.bedrock.policy).Statement[0].Resource == "arn:aws:bedrock:eu-central-1::foundation-model/amazon.titan-embed-text-v2:0" && output.bedrock_region == "eu-central-1"
    error_message = "Cross-region inference must grant the model in the inference region."
  }
}

run "reject_wildcard_principal" {
  command = plan
  variables { client_principal_arn = "*" }
  expect_failures = [var.client_principal_arn]
}

run "reject_invalid_domain" {
  command = plan
  variables { domain_name = "INVALID.domain" }
  expect_failures = [var.domain_name]
}

run "reject_zero_instances" {
  command = plan
  variables { instance_count = 0 }
  expect_failures = [var.instance_count]
}

run "reject_fractional_instances" {
  command = plan
  variables { instance_count = 1.5 }
  expect_failures = [var.instance_count]
}

run "reject_small_volume" {
  command = plan
  variables { volume_size = 1 }
  expect_failures = [var.volume_size]
}

run "reject_sts_session_principal" {
  command = plan
  variables { client_principal_arn = "arn:aws:sts::123456789012:assumed-role/demo/session" }
  expect_failures = [var.client_principal_arn]
}

run "reject_cross_account_client" {
  command = plan
  variables { client_principal_arn = "arn:aws:iam::999999999999:role/client" }
  expect_failures = [aws_opensearch_domain.demo]
}
