output "opensearch_endpoint" {
  description = "AWS_OPENSEARCH_ENDPOINT for the existing Docker example."
  value       = "https://${aws_opensearch_domain.demo.endpoint}"
}

output "bedrock_role_arn" {
  description = "AWS_BEDROCK_ROLE_ARN used by the OpenSearch remote model connector."
  value       = aws_iam_role.bedrock.arn
}

output "kafka_role_arn" {
  description = "AWS_CONNECT_ROLE_ARN for the native Aiven sink's STS provider."
  value       = aws_iam_role.kafka.arn
}

output "aws_region" {
  description = "AWS_DEFAULT_REGION and extension signing region."
  value       = var.aws_region
}

output "bedrock_region" {
  description = "AWS_BEDROCK_REGION for remote inference."
  value       = local.bedrock_region
}

output "domain_arn" {
  description = "Domain ARN for inspection and IAM troubleshooting."
  value       = local.domain_arn
}

output "client_principal_arn" {
  description = "Resolved IAM client identity; preserve this input if provisioning credentials change."
  value       = local.client_principal_arn
}
