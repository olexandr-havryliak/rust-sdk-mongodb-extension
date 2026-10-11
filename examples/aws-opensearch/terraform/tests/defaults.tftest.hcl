mock_provider "aws" {
  mock_data "aws_caller_identity" {
    defaults = { account_id = "123456789012", arn = "arn:aws:iam::123456789012:user/development/demo" }
  }
  mock_data "aws_partition" {
    defaults = { partition = "aws" }
  }
  mock_data "aws_iam_session_context" {
    defaults = { issuer_arn = "arn:aws:iam::123456789012:user/development/demo" }
  }
}

run "automatic_user_and_defaults" {
  command = plan
  assert {
    condition     = aws_iam_user_policy_attachment.client[0].user == "demo" && aws_opensearch_domain.demo.domain_name == "mongodb-vector-demo" && output.aws_region == "us-east-1"
    error_message = "Credentials alone must select the current IAM user and default domain/region."
  }
  assert {
    condition     = jsondecode(aws_iam_role.kafka.assume_role_policy).Statement[0].Principal.AWS == "arn:aws:iam::123456789012:user/development/demo"
    error_message = "Kafka must trust the automatically resolved IAM user."
  }
  assert {
    condition     = output.client_principal_arn == "arn:aws:iam::123456789012:user/development/demo"
    error_message = "Expose the selected client so subsequent runs can retain it when credentials change."
  }
}

run "automatic_session_preserves_role_path" {
  command = plan
  override_data {
    target = data.aws_caller_identity.current
    values = { account_id = "123456789012", arn = "arn:aws:sts::123456789012:assumed-role/demo/session" }
  }
  override_data {
    target = data.aws_iam_session_context.client[0]
    values = { issuer_arn = "arn:aws:iam::123456789012:role/development/demo" }
  }
  assert {
    condition     = aws_iam_role_policy_attachment.client[0].role == "demo" && jsondecode(aws_iam_role.kafka.assume_role_policy).Statement[0].Principal.AWS == "arn:aws:iam::123456789012:role/development/demo"
    error_message = "Use the original IAM role ARN, retaining its path, not the STS ARN."
  }
}

run "manual_principal_skips_discovery" {
  command = plan
  variables {
    client_principal_arn = "arn:aws:iam::123456789012:role/other/client"
    aws_region           = "eu-west-1"
    domain_name          = "manual-demo"
  }
  assert {
    condition     = length(data.aws_iam_session_context.client) == 0 && aws_iam_role_policy_attachment.client[0].role == "client" && output.aws_region == "eu-west-1" && aws_opensearch_domain.demo.domain_name == "manual-demo"
    error_message = "Explicit inputs must override automatic defaults without session discovery."
  }
}

run "reject_automatic_root" {
  command = plan
  override_data {
    target = data.aws_iam_session_context.client[0]
    values = { issuer_arn = "arn:aws:iam::123456789012:root" }
  }
  expect_failures = [aws_opensearch_domain.demo]
}

run "reject_automatic_federated_user" {
  command = plan
  override_data {
    target = data.aws_iam_session_context.client[0]
    values = { issuer_arn = "arn:aws:sts::123456789012:federated-user/demo" }
  }
  expect_failures = [aws_opensearch_domain.demo]
}
