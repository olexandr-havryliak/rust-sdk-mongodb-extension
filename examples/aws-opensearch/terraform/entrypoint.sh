#!/bin/sh
set -eu

export TF_VAR_aws_region="${TF_VAR_aws_region:-${AWS_DEFAULT_REGION:-us-east-1}}"
exec terraform "$@"
