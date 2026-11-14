variable "region" {
  description = "The AWS region. Pick the one closest to everyone who'll use the server: typing goes there and back."
  type        = string
  default     = "me-central-1"
}

variable "name" {
  description = "The machine's name, which is also its name on the tailnet, so the app connects to http://<name>:7420."
  type        = string
  default     = "nodus"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,30}[a-z0-9]$", var.name))
    error_message = "Use lowercase letters, digits and hyphens, starting with a letter."
  }
}

variable "instance_type" {
  description = "4 vCPU and 16 GB suit two people running several sessions with Docker builds. Graviton (m7g) or Intel/AMD (m7i) both work; avoid burstable t types."
  type        = string
  default     = "m7g.xlarge"
}

variable "disk_gb" {
  description = "The disk. Each worktree of a project costs its dependencies again (about 1.5 GB for a Laravel and Node app)."
  type        = number
  default     = 100

  validation {
    condition     = var.disk_gb >= 30
    error_message = "Give it at least 30 GB."
  }
}

variable "ssh_public_key" {
  description = "Your SSH public key (the contents of e.g. ~/.ssh/id_ed25519.pub), for user ubuntu."
  type        = string
}

variable "tailscale_auth_key" {
  description = "A single-use, pre-approved Tailscale auth key tagged tag:nodus, so the server joins your tailnet on its own while it boots. Leave empty to join by hand over SSH."
  type        = string
  default     = ""
  sensitive   = true
}

variable "ssh_cidr" {
  description = "Where SSH may come from over the internet, such as \"203.0.113.7/32\". Leave null for none: with Tailscale you SSH over the tailnet instead."
  type        = string
  default     = null

  validation {
    condition     = var.ssh_cidr == null || (can(cidrhost(var.ssh_cidr, 0)) && !endswith(coalesce(var.ssh_cidr, "/32"), "/0"))
    error_message = "Give one address or a small range, such as \"203.0.113.7/32\", never the whole internet."
  }
}

variable "nodus_version" {
  description = "The Nodus release to install, such as \"1.0.18\". Empty for the latest."
  type        = string
  default     = ""
}
