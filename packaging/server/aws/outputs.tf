output "app_address" {
  description = "What to type in Nodus: New Project ▾ → Connect to Server…"
  value       = local.on_tailnet ? "http://${var.name}:7420" : "http://127.0.0.1:7420 (through: ssh -N -L 7420:127.0.0.1:7420 ubuntu@${aws_instance.this.public_ip})"
}

output "ssh" {
  description = "How to reach the machine."
  value       = local.on_tailnet ? "ssh ubuntu@${var.name}" : "ssh ubuntu@${aws_instance.this.public_ip}"
}

output "watch_install" {
  description = "Follow the install (about 5–10 minutes after boot), ending with the setup code."
  value       = "${local.on_tailnet ? "ssh ubuntu@${var.name}" : "ssh ubuntu@${aws_instance.this.public_ip}"} sudo tail -f /var/log/nodus-bootstrap.log"
}

output "new_setup_code" {
  description = "A fresh setup code, if the printed one is used up or lost (it cancels earlier ones)."
  value       = "${local.on_tailnet ? "ssh ubuntu@${var.name}" : "ssh ubuntu@${aws_instance.this.public_ip}"} sudo -u nodus /opt/nodus/bin/nodus setup-code"
}

output "instance_id" {
  value = aws_instance.this.id
}

output "public_ip" {
  value = aws_instance.this.public_ip
}

output "architecture" {
  value = local.arch
}
