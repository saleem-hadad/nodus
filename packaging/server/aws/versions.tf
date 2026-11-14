terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
}

provider "aws" {
  region = var.region

  # Everything this makes carries these, so it's easy to find in the console.
  default_tags {
    tags = {
      Project   = "nodus-server"
      Name      = var.name
      ManagedBy = "terraform"
    }
  }
}
