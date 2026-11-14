# A Nodus server on AWS, with Terraform

Makes one EC2 instance running Nodus, in a VPC of its own, and installs Nodus on its first boot. `terraform destroy` removes all of it. More on running a server in [docs/server.md](../../../docs/server.md).

What it makes:

- a VPC, subnet, internet gateway and route table, used by nothing else;
- a security group that lets nothing in, except SSH from `ssh_cidr` when you set it;
- an SSH key pair from your public key;
- one instance: Ubuntu 24.04, `m7g.xlarge` (4 vCPU, 16 GB) by default, a 100 GB encrypted gp3 disk, IMDSv2 only, and **no IAM role**.

On first boot the instance downloads `install-server.sh` from the release, checks its signature against [`../signing-key.pub.pem`](../signing-key.pub.pem), and runs it with `--tailscale`. That sets up the two users, rootless Docker, the firewall and the services, and joins your tailnet.

## Before you start

1. **Terraform** 1.6 or later, and **AWS credentials** it can use: install the AWS CLI and run `aws configure sso` (or `aws configure` with an access key), or set `AWS_PROFILE`.
2. **Tailscale.** In the [admin console](https://login.tailscale.com/admin/acls/file), add the policy `install-server.sh` prints, as in docs/server.md:
   ```jsonc
   "tagOwners": { "tag:nodus": ["autogroup:admin"] },
   "grants": [
     { "src": ["autogroup:member"], "dst": ["tag:nodus"], "ip": ["tcp:22", "tcp:7420", "tcp:8100-8999"] },
     { "src": ["autogroup:member"], "dst": ["autogroup:member"], "ip": ["*"] }
   ]
   ```
   Then make an auth key under **Settings → Keys**: not reusable, pre-approved, tagged `tag:nodus`. Being single-use, it's worthless once the server has joined, which matters because the instance's user data holds it.
3. Copy `terraform.tfvars.example` to `terraform.tfvars` and fill it in. Git ignores that file, and the state files.

## Make it

```sh
cd packaging/server/aws
terraform init
terraform apply
```

The outputs tell you what to do next:

- `watch_install`: follow the install. It takes 5–10 minutes after boot and ends with the address and a **setup code**.
- `app_address`: what to type in Nodus under **New Project ▾ → Connect to Server…**, then the setup code, then the wizard.
- `new_setup_code`: a fresh code, if you need one.

## Remove it

```sh
terraform destroy
```

This deletes the instance **and its disk**: anything not pushed to git is gone, including sessions' transcripts and the server's members, devices and Claude accounts. Afterwards, also remove the machine from Tailscale's admin console. If you're done with Nodus for good, delete the GitHub App on GitHub too.

## Good to know

- Changing `ami` (a newer Ubuntu) or `user_data` (a new auth key) never replaces a running server: both are ignored after the first boot. Update Nodus from the app (Settings → Servers → Update Server).
- Region: pick the one nearest everyone who uses it. Typing in a session goes there and back.
- Instance type: Graviton (`m7g`) or Intel/AMD (`m7i`) both work; the release has both builds. Avoid burstable `t` types: sessions and builds spend their CPU credits fast.
- Backups: git is the backup for code. A scheduled EBS snapshot (Data Lifecycle Manager) is a cheap extra, not set up here.
