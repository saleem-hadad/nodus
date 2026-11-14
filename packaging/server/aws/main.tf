# A Nodus server on EC2: its own VPC, one instance, nothing else.
# `terraform destroy` removes all of it, the disk included.

locals {
  # Ubuntu's images are named by Debian's architecture names.
  arch = contains(data.aws_ec2_instance_type.this.supported_architectures, "arm64") ? "arm64" : "amd64"
  # The first zone in the region that offers the instance type.
  zone = sort(data.aws_ec2_instance_type_offerings.zones.locations)[0]
  # Whether it joins the tailnet on boot. Only a yes or no, never the key.
  on_tailnet = nonsensitive(var.tailscale_auth_key != "")
}

data "aws_ec2_instance_type" "this" {
  instance_type = var.instance_type
}

data "aws_ec2_instance_type_offerings" "zones" {
  location_type = "availability-zone"
  filter {
    name   = "instance-type"
    values = [var.instance_type]
  }
}

# Canonical's current Ubuntu 24.04 image for this architecture.
data "aws_ssm_parameter" "ubuntu" {
  name = "/aws/service/canonical/ubuntu/server/24.04/stable/current/${local.arch}/hvm/ebs-gp3/ami-id"
}

# --------------------------------------------------------------------------
# Network: a VPC of its own, so nothing else in the account is touched.

resource "aws_vpc" "this" {
  cidr_block           = "10.42.0.0/16"
  enable_dns_support   = true
  enable_dns_hostnames = true
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
}

resource "aws_subnet" "this" {
  vpc_id                  = aws_vpc.this.id
  cidr_block              = "10.42.1.0/24"
  availability_zone       = local.zone
  map_public_ip_on_launch = true
}

resource "aws_route_table" "this" {
  vpc_id = aws_vpc.this.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this.id
  }
}

resource "aws_route_table_association" "this" {
  subnet_id      = aws_subnet.this.id
  route_table_id = aws_route_table.this.id
}

# Nothing comes in but SSH, and only from ssh_cidr when it's set. Nodus
# itself (port 7420) and previews are reached over Tailscale, never the
# internet. The server's own firewall (install-server.sh) drops the rest too.
resource "aws_security_group" "this" {
  name        = "${var.name}-server"
  description = "Nodus server: SSH from one address at most; everything else over Tailscale"
  vpc_id      = aws_vpc.this.id
}

resource "aws_vpc_security_group_ingress_rule" "ssh" {
  count             = var.ssh_cidr == null ? 0 : 1
  security_group_id = aws_security_group.this.id
  description       = "SSH from you"
  ip_protocol       = "tcp"
  from_port         = 22
  to_port           = 22
  cidr_ipv4         = var.ssh_cidr
}

# Out to anywhere: apt, GitHub, Anthropic, Docker images, Tailscale.
resource "aws_vpc_security_group_egress_rule" "all" {
  security_group_id = aws_security_group.this.id
  description       = "Everything out"
  ip_protocol       = "-1"
  cidr_ipv4         = "0.0.0.0/0"
}

# --------------------------------------------------------------------------
# The server

resource "aws_key_pair" "this" {
  key_name   = "${var.name}-server"
  public_key = var.ssh_public_key
}

resource "aws_instance" "this" {
  ami                    = data.aws_ssm_parameter.ubuntu.value
  instance_type          = var.instance_type
  subnet_id              = aws_subnet.this.id
  vpc_security_group_ids = [aws_security_group.this.id]
  key_name               = aws_key_pair.this.key_name

  # No IAM role: there are no cloud credentials on the machine for a
  # session to find. IMDSv2 only, and one hop, so containers can't reach it.
  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
  }

  root_block_device {
    volume_type           = "gp3"
    volume_size           = var.disk_gb
    encrypted             = true
    delete_on_termination = true
  }

  # Installs Nodus on first boot: see user-data.sh.tftpl.
  user_data = templatefile("${path.module}/user-data.sh.tftpl", {
    name               = var.name
    nodus_version      = var.nodus_version
    tailscale_auth_key = var.tailscale_auth_key
    signing_key        = trimspace(file("${path.module}/../signing-key.pub.pem"))
  })

  lifecycle {
    # A newer Ubuntu image, or a new auth key, must never replace a server
    # that's in use: both only matter on the first boot.
    ignore_changes = [ami, user_data]

    precondition {
      condition     = local.on_tailnet || var.ssh_cidr != null
      error_message = "Set tailscale_auth_key, ssh_cidr, or both: otherwise nothing can reach the server."
    }
  }
}
