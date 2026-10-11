variable "aws_region" {
  description = "Commercial AWS region for the OpenSearch domain."
  type        = string
  default     = "us-east-1"
  validation {
    condition     = can(regex("^[a-z]{2}-[a-z]+-[1-9][0-9]*$", var.aws_region)) && !startswith(var.aws_region, "cn-")
    error_message = "Use a commercial AWS region, for example us-east-1."
  }
}

variable "bedrock_region" {
  description = "Titan inference region; null uses aws_region."
  type        = string
  default     = null
  validation {
    condition     = var.bedrock_region == null ? true : can(regex("^[a-z]{2}-[a-z]+-[1-9][0-9]*$", var.bedrock_region)) && !startswith(var.bedrock_region, "cn-")
    error_message = "Use a commercial AWS region or null."
  }
}

variable "client_principal_arn" {
  description = "Existing same-account IAM user/role; null discovers the identity from current AWS credentials."
  type        = string
  default     = null
  validation {
    condition     = var.client_principal_arn == null ? true : can(regex("^arn:aws:iam::[0-9]{12}:(user|role)/[A-Za-z0-9_+=,.@/-]+$", var.client_principal_arn))
    error_message = "Provide an explicit commercial AWS IAM user or role ARN; wildcards and STS session ARNs are not supported."
  }
}

variable "domain_name" {
  description = "Unique new domain name; use a different domain for each MongoDB topology."
  type        = string
  default     = "mongodb-vector-demo"
  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,27}$", var.domain_name))
    error_message = "Domain names must be 3-28 lowercase letters/digits/hyphens, starting with a letter."
  }
}

variable "instance_type" {
  description = "EBS-capable OpenSearch instance supported in the selected region; default is a small non-HA PoC."
  type        = string
  default     = "t3.small.search"
  validation {
    condition     = can(regex("^[a-z][a-z0-9]*\\.[a-z0-9]+\\.search$", var.instance_type))
    error_message = "Use an OpenSearch instance type such as t3.small.search."
  }
}

variable "instance_count" {
  description = "Number of data nodes; increasing this alone does not configure a complete HA deployment."
  type        = number
  default     = 1
  validation {
    condition     = var.instance_count >= 1 && var.instance_count <= 10 && floor(var.instance_count) == var.instance_count
    error_message = "Use an integer between 1 and 10 data nodes for this PoC configuration."
  }
}

variable "volume_size" {
  description = "gp3 volume size per data node in GiB; verify instance-specific AWS limits."
  type        = number
  default     = 10
  validation {
    condition     = var.volume_size >= 10 && floor(var.volume_size) == var.volume_size
    error_message = "Use an integer volume size of at least 10 GiB."
  }
}
