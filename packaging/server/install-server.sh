#!/usr/bin/env bash
# shellcheck disable=SC2317 # every function runs through main, step or run
# Installs a Nodus server (spec SRV-010…013), or upgrades one in place.
#
#   curl -fsSL https://github.com/saleem-hadad/nodus/releases/latest/download/install-server.sh | sudo bash -s -- [--tailscale]
#
# Run as root on Ubuntu 22.04+ or Debian 12+, x64 or arm64, kernel 5.11+.
# It stops at the first step that fails, says which, and leaves the log in
# /var/log/nodus-install.log. Every step is safe to run again: running the
# script again upgrades the server and keeps its data.
#
# Options:
#   --tailscale            also install Tailscale and join the tailnet as tag:nodus (SRV-029)
#   --version <X.Y.Z>      install that release instead of the latest
#   --uninstall            stop and remove the services and the binary; keep the data
#   --purge                with --uninstall: also remove the users, their data, their
#                          Docker and the firewall
#   --local-binary <path>  FOR TESTING ONLY: install this binary, unsigned
#   -h, --help             this text
#
# Environment: NODUS_RELEASE_URL downloads the release files from another
# folder (a mirror); the signature is checked all the same. TS_AUTHKEY joins
# Tailscale with that key instead of a login link.
#
# What it sets up:
#   nodus       runs `nodus serve` (nodus.service) and keeps the server's
#               secrets in /home/nodus (0700). Member of the nodus-run group.
#   nodus-run   runs the session host (nodus-host.service) and everything that
#               touches repo content: claude, git, terminals, rootless Docker.
#               Owns /home/nodus-run (0750).
#   /opt/nodus/bin/nodus           the server binary, owned by nodus, 0755
#   /srv/nodus/uploads             2770 nodus:nodus-run
#   /etc/nodus/nftables.nft        the firewall, loaded by nodus-firewall.service

main() {
	set -Eeuo pipefail
	umask 022
	export LC_ALL=C
	export DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=a

	readonly REPO="saleem-hadad/nodus"
	readonly PORT=7420
	readonly SERVE_USER="nodus" RUN_USER="nodus-run"
	readonly SERVE_HOME="/home/nodus" RUN_HOME="/home/nodus-run"
	readonly BIN_DIR="/opt/nodus/bin" BIN="/opt/nodus/bin/nodus"
	readonly UPLOADS="/srv/nodus/uploads"
	readonly HOST_SOCKET="/run/nodus/host.sock"
	readonly UNIT_DIR="/etc/systemd/system"
	readonly FIREWALL_FILE="/etc/nodus/nftables.nft"
	readonly LOG="/var/log/nodus-install.log"

	STEP="Starting"
	TAILSCALE=0 UNINSTALL=0 PURGE=0 RELEASE="" LOCAL_BINARY=""
	parse_args "$@"

	trap 'on_error $LINENO' ERR
	WORK=$(mktemp -d)
	trap 'rm -rf "$WORK"' EXIT

	[ "$(id -u)" -eq 0 ] || fail "Run it as root: curl -fsSL …/install-server.sh | sudo bash"
	touch "$LOG" && chmod 0600 "$LOG"
	printf '\n=== install-server.sh %s, %s\n' "$*" "$(date -u +%FT%TZ)" >>"$LOG"

	if [ "$UNINSTALL" = 1 ]; then
		uninstall
		return
	fi

	step 1 "Checking the system" check_system
	step 2 "Creating the users and folders" create_users
	step 3 "Installing git, curl, uidmap, dbus-user-session, gh and nftables" install_packages
	step 4 "Installing Claude Code for $RUN_USER" install_claude
	step 5 "Installing rootless Docker for $RUN_USER" install_docker
	step 6 "Installing the Nodus server" install_binary
	step 7 "Turning on the firewall" install_firewall
	step 8 "Starting nodus-host.service and nodus.service" install_units
	if [ "$TAILSCALE" = 1 ]; then
		step 9 "Joining Tailscale" join_tailscale
	fi
	STEP="Finishing"
	finish
}

# --------------------------------------------------------------------------
# Plumbing

usage() {
	sed -n '3,/^$/s/^# \{0,1\}//p' "${BASH_SOURCE[0]}" 2>/dev/null ||
		echo "Usage: install-server.sh [--tailscale] [--version X.Y.Z] [--uninstall [--purge]] [--local-binary <path>]"
}

parse_args() {
	while [ $# -gt 0 ]; do
		case "$1" in
		--tailscale) TAILSCALE=1 ;;
		--uninstall) UNINSTALL=1 ;;
		--purge) PURGE=1 ;;
		--version)
			[ $# -ge 2 ] || fail "--version needs a version, such as 1.1.0"
			RELEASE=${2#v}
			shift
			;;
		--version=*) RELEASE=${1#--version=} RELEASE=${RELEASE#v} ;;
		--local-binary)
			[ $# -ge 2 ] || fail "--local-binary needs a path"
			LOCAL_BINARY=$2
			shift
			;;
		--local-binary=*) LOCAL_BINARY=${1#--local-binary=} ;;
		-h | --help)
			usage
			exit 0
			;;
		*) fail "Unknown option $1 (try --help)" ;;
		esac
		shift
	done
	if [ -n "$RELEASE" ] && ! [[ "$RELEASE" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
		fail "--version takes three numbers, such as 1.1.0, not $RELEASE"
	fi
	if [ "$PURGE" = 1 ] && [ "$UNINSTALL" = 0 ]; then
		fail "--purge goes with --uninstall: install-server.sh --uninstall --purge"
	fi
	if [ -n "$LOCAL_BINARY" ] && [ -n "$RELEASE" ]; then
		fail "--local-binary and --version don't go together"
	fi
	return 0
}

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
warn() { printf '\n!!  %s\n\n' "$*" >&2; }

# Stops with a plain message.
fail() {
	trap - ERR
	printf '\ninstall-server.sh: %s\n' "$*" >&2
	exit 1
}

# A command that failed under `set -e`: which step, and the end of the log.
on_error() {
	local code=$? line=$1
	trap - ERR
	# In a subshell ($(…), as_user): the shell that started it says so.
	if [ "$BASH_SUBSHELL" -gt 0 ]; then
		exit "$code"
	fi
	printf '\ninstall-server.sh: stopped. %s failed (exit %s, line %s).\n' "$STEP" "$code" "$line" >&2
	if [ -s "$LOG" ]; then
		printf 'The last lines of %s:\n' "$LOG" >&2
		tail -n 15 "$LOG" | sed 's/^/  /' >&2
	fi
	exit "$code"
}

step() {
	local number=$1 title=$2
	shift 2
	STEP="Step $number ($title)"
	say "$number. $title"
	printf '\n--- %s\n' "$STEP" >>"$LOG"
	"$@"
}

# Runs a command with its output in the log.
run() {
	printf '+ %s\n' "$*" >>"$LOG"
	"$@" </dev/null >>"$LOG" 2>&1
}

apt_install() {
	run apt-get -y -q -o DPkg::Lock::Timeout=300 install --no-install-recommends "$@"
}

apt_update() {
	run apt-get -q -o DPkg::Lock::Timeout=300 update
}

# Runs a command as `user`, in its home, with a clean environment, and its
# user manager's runtime folder when it has one (for systemctl --user).
as_user() {
	local user=$1 home uid
	shift
	home=$(getent passwd "$user" | cut -d: -f6)
	uid=$(id -u "$user")
	local env=(HOME="$home" USER="$user" LOGNAME="$user" SHELL=/bin/bash LANG=C.UTF-8
		PATH="$home/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
	if [ -d "/run/user/$uid" ]; then
		env+=(XDG_RUNTIME_DIR="/run/user/$uid" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus")
	fi
	(cd "$home" && runuser -u "$user" -- env -i "${env[@]}" "$@")
}

# Writes `content` to `path` with `mode` when it differs; true when it changed.
write_file() {
	local path=$1 mode=$2 content=$3
	if [ -f "$path" ] && [ "$(cat "$path")" = "$content" ]; then
		chmod "$mode" "$path"
		return 1
	fi
	printf '%s\n' "$content" >"$path.new"
	chmod "$mode" "$path.new"
	mv -f "$path.new" "$path"
}

version_at_least() { # have want
	[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | sed -n 1p)" = "$2" ]
}

# --------------------------------------------------------------------------
# 1. The system

check_system() {
	[ -d /run/systemd/system ] || fail "This machine doesn't run systemd, which Nodus's services need."
	[ -r /etc/os-release ] || fail "Can't tell which Linux this is: there's no /etc/os-release."
	# shellcheck disable=SC1091
	OS_ID=$(. /etc/os-release && echo "${ID:-}")
	# shellcheck disable=SC1091
	OS_VERSION=$(. /etc/os-release && echo "${VERSION_ID:-}")
	# shellcheck disable=SC1091
	OS_CODENAME=$(. /etc/os-release && echo "${VERSION_CODENAME:-}")
	case "$OS_ID" in
	ubuntu) version_at_least "$OS_VERSION" 22.04 || fail "Ubuntu $OS_VERSION is too old: Nodus needs Ubuntu 22.04 or later." ;;
	debian) version_at_least "$OS_VERSION" 12 || fail "Debian $OS_VERSION is too old: Nodus needs Debian 12 or later." ;;
	*) fail "Nodus installs on Ubuntu 22.04+ or Debian 12+, not on ${OS_ID:-this system}." ;;
	esac
	[ -n "$OS_CODENAME" ] || fail "Can't tell this $OS_ID release's codename from /etc/os-release."

	case "$(uname -m)" in
	x86_64) ARCH=x64 ;;
	aarch64 | arm64) ARCH=arm64 ;;
	*) fail "Nodus runs on x64 or arm64 machines, not $(uname -m)." ;;
	esac
	DPKG_ARCH=$(dpkg --print-architecture)

	local kernel
	kernel=$(uname -r | grep -oE '^[0-9]+\.[0-9]+')
	version_at_least "$kernel" 5.11 || fail "Linux $kernel is too old: rootless Docker needs kernel 5.11 or later."
	note "$OS_ID $OS_VERSION ($OS_CODENAME), $ARCH, kernel $(uname -r)"
}

# --------------------------------------------------------------------------
# 2. Users and folders (SRV-003, SRV-005)

# A user with its own group, a home and a real shell: systemd sets SHELL
# from the passwd entry, and terminal cards start that shell, so a nologin
# shell would break them.
ensure_user() {
	local user=$1 home=$2 mode=$3
	if ! getent group "$user" >/dev/null; then
		run groupadd "$user"
	fi
	if ! getent passwd "$user" >/dev/null; then
		run useradd --create-home --home-dir "$home" --gid "$user" --shell /bin/bash \
			--comment "Nodus server" "$user"
	else
		run usermod --shell /bin/bash "$user"
	fi
	[ "$(getent passwd "$user" | cut -d: -f6)" = "$home" ] ||
		fail "The user $user already exists with another home than $home; remove it or move its home first."
	install -d -o "$user" -g "$user" -m "$mode" "$home"
	# Rootless Docker maps its containers' users into these ranges.
	ensure_subids "$user" /etc/subuid --add-subuids
	ensure_subids "$user" /etc/subgid --add-subgids
}

ensure_subids() {
	local user=$1 file=$2 flag=$3 start
	touch "$file"
	grep -q "^$user:" "$file" && return 0
	start=$(awk -F: 'BEGIN { end = 100000 } $2 + $3 > end { end = $2 + $3 } END { print end }' "$file")
	run usermod "$flag" "$start-$((start + 65535))" "$user"
}

create_users() {
	ensure_user "$RUN_USER" "$RUN_HOME" 0750
	ensure_user "$SERVE_USER" "$SERVE_HOME" 0700
	# nodus reads what nodus-run writes (transcripts, worktrees), never the other way.
	run usermod --append --groups "$RUN_USER" "$SERVE_USER"

	install -d -o root -g root -m 0755 /srv/nodus
	install -d -o "$SERVE_USER" -g "$RUN_USER" -m 2770 "$UPLOADS"
	install -d -o root -g root -m 0755 /opt/nodus
	# nodus can replace the binary (updates, SRV-200); nodus-run can only run it.
	install -d -o "$SERVE_USER" -g "$SERVE_USER" -m 0755 "$BIN_DIR"
	note "$SERVE_USER (uid $(id -u "$SERVE_USER")), $RUN_USER (uid $(id -u "$RUN_USER")), $SERVE_USER in group $RUN_USER"
}

# --------------------------------------------------------------------------
# 3. Packages

install_packages() {
	apt_update
	apt_install ca-certificates curl gnupg openssl git uidmap dbus-user-session nftables
	# gh, from GitHub's own apt repository.
	install -d -m 0755 /etc/apt/keyrings
	run curl -fsSL -o /etc/apt/keyrings/githubcli-archive-keyring.gpg \
		https://cli.github.com/packages/githubcli-archive-keyring.gpg
	chmod 0644 /etc/apt/keyrings/githubcli-archive-keyring.gpg
	if write_file /etc/apt/sources.list.d/github-cli.list 0644 \
		"deb [arch=$DPKG_ARCH signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" ||
		! dpkg -s gh >/dev/null 2>&1; then
		apt_update
	fi
	apt_install gh
	note "git $(git --version | cut -d' ' -f3), gh $(gh --version | awk 'NR == 1 { print $3 }'), $(nft --version)"
}

# --------------------------------------------------------------------------
# 4. Claude Code, with Anthropic's own installer, as nodus-run

install_claude() {
	if [ -x "$RUN_HOME/.local/bin/claude" ]; then
		note "Already installed; it updates itself."
	else
		run as_user "$RUN_USER" bash -o pipefail -c 'curl -fsSL https://claude.ai/install.sh | bash'
	fi
	local version
	version=$(as_user "$RUN_USER" "$RUN_HOME/.local/bin/claude" --version 2>>"$LOG") ||
		fail "Claude Code didn't install for $RUN_USER: $RUN_HOME/.local/bin/claude --version failed."
	note "claude $version"
}

# --------------------------------------------------------------------------
# 5. Rootless Docker for nodus-run

install_docker() {
	local uid
	uid=$(id -u "$RUN_USER")

	install -d -m 0755 /etc/apt/keyrings
	run curl -fsSL -o /etc/apt/keyrings/docker.asc "https://download.docker.com/linux/$OS_ID/gpg"
	chmod 0644 /etc/apt/keyrings/docker.asc
	if write_file /etc/apt/sources.list.d/docker.list 0644 \
		"deb [arch=$DPKG_ARCH signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/$OS_ID $OS_CODENAME stable" ||
		! dpkg -s docker-ce-rootless-extras >/dev/null 2>&1; then
		apt_update
	fi

	# The packages start the system-wide (root) daemon. Only rootless Docker
	# is wanted: root's daemon publishes ports past the firewall's input
	# chain, and its socket is root for anyone who reaches it. So it isn't
	# started while installing, and it's turned off after, unless this
	# machine already ran it, which is the admin's call.
	local rootful=0 policy_made=0
	if systemctl is-active --quiet docker.service || systemctl is-active --quiet docker.socket; then
		rootful=1
	fi
	if [ ! -e /usr/sbin/policy-rc.d ]; then
		printf '#!/bin/sh\nexit 101\n' >/usr/sbin/policy-rc.d
		chmod 0755 /usr/sbin/policy-rc.d
		policy_made=1
	fi
	local installed=0
	apt_install docker-ce docker-ce-cli containerd.io docker-ce-rootless-extras \
		docker-buildx-plugin docker-compose-plugin slirp4netns && installed=1
	if [ "$policy_made" = 1 ]; then rm -f /usr/sbin/policy-rc.d; fi
	[ "$installed" = 1 ] || fail "Installing Docker's packages failed; see $LOG."
	if [ "$rootful" = 1 ]; then
		warn "The system-wide Docker daemon was already running here, so it's left on. Ports its containers publish skip the firewall's input rules; only rootless Docker (as $RUN_USER) is covered."
	else
		run systemctl disable --now docker.service docker.socket containerd.service
	fi

	allow_rootlesskit_userns

	# nodus-run's own systemd, running without a login, which rootless
	# Docker lives in.
	run loginctl enable-linger "$RUN_USER"
	run systemctl start "user@$uid.service"
	local tries=0
	until [ -S "/run/user/$uid/bus" ]; do
		tries=$((tries + 1))
		[ "$tries" -le 20 ] || fail "$RUN_USER's user manager has no D-Bus socket (/run/user/$uid/bus); is dbus-user-session installed?"
		if [ "$tries" = 10 ]; then run systemctl restart "user@$uid.service"; fi
		sleep 0.5
	done

	if as_user "$RUN_USER" systemctl --user is-active --quiet docker.service; then
		note "Already set up."
	else
		run as_user "$RUN_USER" dockerd-rootless-setuptool.sh install
	fi
	run as_user "$RUN_USER" systemctl --user enable docker.service
	tries=0
	until as_user "$RUN_USER" env DOCKER_HOST="unix:///run/user/$uid/docker.sock" docker info >/dev/null 2>&1; do
		tries=$((tries + 1))
		[ "$tries" -le 30 ] || fail "Rootless Docker for $RUN_USER doesn't answer at /run/user/$uid/docker.sock."
		sleep 1
	done
	note "$(as_user "$RUN_USER" env DOCKER_HOST="unix:///run/user/$uid/docker.sock" docker version --format 'Docker {{.Server.Version}}, rootless') at /run/user/$uid/docker.sock"
}

# Ubuntu 23.10+ lets programs make user namespaces only when AppArmor says
# so, and rootless Docker's rootlesskit needs one. Docker's documented fix:
# a profile that allows it, for that one binary.
allow_rootlesskit_userns() {
	local knob=/proc/sys/kernel/apparmor_restrict_unprivileged_userns
	[ -r "$knob" ] && [ "$(cat "$knob")" = 1 ] || return 0
	command -v apparmor_parser >/dev/null || return 0
	local rootlesskit profile
	rootlesskit=$(command -v rootlesskit) || fail "docker-ce-rootless-extras installed no rootlesskit."
	# Ubuntu 24.04's apparmor package has one for /usr/bin/rootlesskit.
	if grep -ls -- "$rootlesskit" /etc/apparmor.d/* | xargs -r grep -ls "userns," | grep -q .; then
		return 0
	fi
	profile="/etc/apparmor.d/$(echo "${rootlesskit#/}" | tr / .)"
	write_file "$profile" 0644 "# Written by Nodus's install-server.sh: rootless Docker needs a user namespace.
abi <abi/4.0>,
include <tunables/global>

\"$rootlesskit\" flags=(unconfined) {
  userns,

  include if exists <local/$(basename "$profile")>
}" || true
	run apparmor_parser -r "$profile"
	note "AppArmor: allowed $rootlesskit a user namespace ($profile)"
}

# --------------------------------------------------------------------------
# 6. The binary, with its signature checked (SRV-201)

# The release's public signing key, ed25519. Releases sign the server and
# this script with its private key, kept in the repo's secrets as
# NODUS_SERVER_SIGNING_KEY; scripts/server-signing-key.sh writes it here.
# BEGIN SIGNING KEY
# TODO: placeholder. Run scripts/server-signing-key.sh to make the key and write it here.
readonly SIGNING_KEY=''
# END SIGNING KEY

install_binary() {
	local new="$WORK/nodus"
	if [ -n "$LOCAL_BINARY" ]; then
		[ -f "$LOCAL_BINARY" ] || fail "--local-binary: there's no file at $LOCAL_BINARY"
		warn "TESTING ONLY: installing $LOCAL_BINARY as it is. It was NOT downloaded from a release and its signature was NOT checked."
		cp "$LOCAL_BINARY" "$new"
	else
		download_binary "$new"
	fi
	chmod 0755 "$new"
	# A quick look that it runs on this machine at all (it prints its usage).
	local usage
	usage=$("$new" 2>&1 || true)
	if ! grep -q "Usage" <<<"$usage"; then
		fail "The server binary doesn't run on this machine ($(file -b "$new" 2>/dev/null || echo "unknown file"))."
	fi

	BINARY_CHANGED=1
	if [ -f "$BIN" ] && cmp -s "$new" "$BIN"; then
		BINARY_CHANGED=0
		note "$BIN is up to date."
	else
		# Beside the old one, then renamed over it: a running server keeps
		# its own copy until it restarts.
		install -o "$SERVE_USER" -g "$SERVE_USER" -m 0755 "$new" "$BIN.new"
		mv -f "$BIN.new" "$BIN"
		note "Installed $BIN"
	fi
}

download_binary() {
	local out=$1 base asset="nodus-server-linux-$ARCH.tar.gz"
	[ -n "$SIGNING_KEY" ] ||
		fail "This copy of install-server.sh has no signing key yet, so it can't check a download. Use a script from a release."
	command -v openssl >/dev/null || fail "openssl is missing."
	local openssl_version
	openssl_version=$(openssl version | awk '{print $2}')
	version_at_least "$openssl_version" 3.0 || fail "OpenSSL $openssl_version can't check ed25519 signatures; 3.0 or later can."

	if [ -n "${NODUS_RELEASE_URL:-}" ]; then
		base=${NODUS_RELEASE_URL%/}
	elif [ -n "$RELEASE" ]; then
		base="https://github.com/$REPO/releases/download/v$RELEASE"
	else
		# The latest release's tag, once, so the archive and its signature
		# come from the same release even if a new one is published meanwhile.
		local latest
		latest=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
			fail "Can't reach GitHub to find the latest Nodus release."
		RELEASE=${latest##*/v}
		[[ "$RELEASE" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Can't tell the latest release's version from $latest."
		base="https://github.com/$REPO/releases/download/v$RELEASE"
	fi
	note "Downloading $asset${RELEASE:+ (v$RELEASE)}"
	local file
	for file in "$asset" "$asset.sig" "$asset.sha256"; do
		curl -fsSL --retry 3 -o "$WORK/$file" "$base/$file" 2>>"$LOG" ||
			fail "Can't download $base/$file. That release may have no Linux server; pick another with --version."
	done
	# The checksum only catches a broken download. The signature is what's
	# trusted, since this runs as root.
	local want got
	want=$(awk '{print $1; exit}' "$WORK/$asset.sha256")
	got=$(sha256sum "$WORK/$asset" | awk '{print $1}')
	[ "$want" = "$got" ] || fail "$asset is damaged (its SHA-256 is $got, the release says $want). Run the script again."
	printf '%s\n' "$SIGNING_KEY" >"$WORK/signing-key.pem"
	openssl pkeyutl -verify -pubin -inkey "$WORK/signing-key.pem" -rawin \
		-in "$WORK/$asset" -sigfile "$WORK/$asset.sig" >>"$LOG" 2>&1 ||
		fail "$asset's signature doesn't match Nodus's signing key. Nothing was installed."
	note "Signature checked."
	mkdir "$WORK/unpacked"
	tar -xzf "$WORK/$asset" -C "$WORK/unpacked" --no-same-owner nodus ||
		fail "$asset has no nodus binary in it."
	if [ ! -f "$WORK/unpacked/nodus" ] || [ -L "$WORK/unpacked/nodus" ]; then
		fail "$asset's nodus isn't a plain file."
	fi
	mv "$WORK/unpacked/nodus" "$out"
}

# --------------------------------------------------------------------------
# 7. The firewall (SRV-011 step 7)

# Its own table and unit, so the admin's /etc/nftables.conf (which starts
# with `flush ruleset`) stays theirs: the unit loads after it.
install_firewall() {
	local serve_uid run_uid ssh_ports nft
	serve_uid=$(id -u "$SERVE_USER")
	run_uid=$(id -u "$RUN_USER")
	nft=$(command -v nft)
	# SSH on 22, and wherever sshd listens besides, so nobody is locked out.
	ssh_ports=$({
		echo 22
		{ sshd -T 2>/dev/null || true; } | awk '$1 == "port" { print $2 }'
	} | sort -un | paste -sd, - | sed 's/,/, /g')

	install -d -m 0755 /etc/nodus
	local changed=0
	write_file "$FIREWALL_FILE" 0644 "#!$nft -f
# Nodus's firewall, written by install-server.sh (SRV-011): running the script
# again rewrites it. Loaded at boot by nodus-firewall.service.

# Replaces the table whole, in one go.
table inet nodus
delete table inet nodus

table inet nodus {
	chain input {
		type filter hook input priority filter; policy drop;
		iif \"lo\" accept
		ct state established,related accept
		ct state invalid drop
		iifname \"tailscale0\" accept
		tcp dport { $ssh_ports } accept
		# IPv6 can't work without neighbour discovery, nor DHCPv6 without its replies.
		icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert, nd-router-advert } ip6 hoplimit 255 accept
		ip6 saddr fe80::/10 udp sport 547 udp dport 546 accept
	}

	# Sessions can't read the cloud instance's credentials: $SERVE_USER ($serve_uid)
	# and $RUN_USER ($run_uid), whose rootless Docker's traffic leaves through
	# its own processes too, never reach the metadata service.
	chain output {
		type filter hook output priority filter; policy accept;
		meta skuid { $serve_uid, $run_uid } ip daddr 169.254.169.254 drop
		meta skuid { $serve_uid, $run_uid } ip6 daddr fd00:ec2::254 drop
	}
}" && changed=1
	run "$nft" -c -f "$FIREWALL_FILE" || fail "nft refused $FIREWALL_FILE; see $LOG."

	write_file "$UNIT_DIR/nodus-firewall.service" 0644 "[Unit]
Description=Nodus firewall (nftables table inet nodus)
Documentation=https://github.com/$REPO
Wants=network-pre.target
Before=network-pre.target shutdown.target
After=nftables.service
Conflicts=shutdown.target
DefaultDependencies=no

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=$nft -f $FIREWALL_FILE
ExecReload=$nft -f $FIREWALL_FILE
ExecStop=$nft delete table inet nodus

[Install]
WantedBy=sysinit.target" && changed=1
	run systemctl daemon-reload
	run systemctl enable nodus-firewall.service
	if systemctl is-active --quiet nodus-firewall.service; then
		# Reloading swaps the table in one step, with no gap.
		if [ "$changed" = 1 ]; then run systemctl reload nodus-firewall.service; fi
	else
		run systemctl start nodus-firewall.service
	fi
	"$nft" list table inet nodus >/dev/null 2>&1 || fail "The firewall's table isn't loaded."
	note "Incoming: only lo, tailscale0, SSH (tcp $ssh_ports) and replies. Metadata service blocked for $SERVE_USER and $RUN_USER."
}

# --------------------------------------------------------------------------
# 8. The services (SRV-013)

install_units() {
	local run_uid host_changed=0 serve_changed=0
	run_uid=$(id -u "$RUN_USER")

	# Absolute paths: in a system unit %h is root's home whatever User= says.
	write_file "$UNIT_DIR/nodus-host.service" 0644 "[Unit]
Description=Nodus session host (sessions, terminals, git and every other process, as $RUN_USER)
Documentation=https://github.com/$REPO
After=network-online.target user@$run_uid.service
Wants=network-online.target

[Service]
User=$RUN_USER
Group=$RUN_USER
WorkingDirectory=$RUN_HOME
ExecStart=$BIN host --socket $HOST_SOCKET
# /run/nodus: 0750 $RUN_USER:$RUN_USER; the host makes the socket 0660.
RuntimeDirectory=nodus
RuntimeDirectoryMode=0750
RuntimeDirectoryPreserve=yes
# Transcripts and worktrees readable by the group, so $SERVE_USER can read them.
UMask=0027
Environment=DOCKER_HOST=unix:///run/user/$run_uid/docker.sock
Restart=always
RestartSec=1
# Restarting the host's own process leaves the sessions it started running.
KillMode=process

[Install]
WantedBy=multi-user.target" && host_changed=1

	write_file "$UNIT_DIR/nodus.service" 0644 "[Unit]
Description=Nodus server
Documentation=https://github.com/$REPO
After=network-online.target nodus-host.service tailscaled.service
Wants=network-online.target nodus-host.service

[Service]
User=$SERVE_USER
Group=$SERVE_USER
SupplementaryGroups=$RUN_USER
WorkingDirectory=$SERVE_HOME
ExecStart=$BIN serve --host-socket $HOST_SOCKET --run-home $RUN_HOME
# Uploads in $UPLOADS are read by sessions; the data folder itself is 0700.
UMask=0027
Restart=always
RestartSec=1

[Install]
WantedBy=multi-user.target" && serve_changed=1

	run systemctl daemon-reload
	run systemctl enable nodus-host.service nodus.service
	if [ "$host_changed" = 1 ] && systemctl is-active --quiet nodus-host.service; then
		note "The session host's unit changed, so it restarts: sessions stop and resume when opened."
		run systemctl restart nodus-host.service
	else
		# A new binary doesn't restart the host: sessions keep running, and
		# the host leaves by itself when the new server speaks another
		# protocol (SRV-004).
		run systemctl start nodus-host.service
	fi
	if [ "$BINARY_CHANGED" = 1 ] || [ "$serve_changed" = 1 ]; then
		run systemctl restart nodus.service
	else
		run systemctl start nodus.service
	fi
	wait_until_up
}

wait_until_up() {
	local tries=0
	until [ -S "$HOST_SOCKET" ] && listening_on >/dev/null; do
		tries=$((tries + 1))
		if [ "$tries" -gt 30 ]; then
			journalctl -u nodus-host.service -u nodus.service -n 20 --no-pager >>"$LOG" 2>&1 || true
			fail "The services didn't come up in 30 seconds. See: journalctl -u nodus-host -u nodus"
		fi
		sleep 1
	done
	note "The session host is at $HOST_SOCKET, and the server at $(listening_on)."
}

# Where nodus serve listens, as `ip:port`.
listening_on() {
	ss -Hltn "sport = :$PORT" | awk '{print $4; exit}' | grep .
}

# --------------------------------------------------------------------------
# 9. Tailscale (SRV-029)

join_tailscale() {
	if ! command -v tailscale >/dev/null; then
		run bash -o pipefail -c 'curl -fsSL https://tailscale.com/install.sh | sh'
	fi
	run systemctl enable --now tailscaled.service

	cat <<'EOF'

    Before joining, add this to your tailnet's policy file
    (https://login.tailscale.com/admin/acls/file), so the server may be
    tagged tag:nodus, members reach it, and it reaches no one:

      "tagOwners": {
        "tag:nodus": ["autogroup:admin"]
      },
      "grants": [
        // Members' devices reach the server: SSH, Nodus, and the previews.
        { "src": ["autogroup:member"], "dst": ["tag:nodus"], "ip": ["tcp:22", "tcp:7420", "tcp:8100-8999"] },
        // Members reach each other. This replaces the default rule that
        // lets everything reach everything: tag:nodus is in no "src", so
        // the server, which runs Claude in auto mode, reaches nothing.
        { "src": ["autogroup:member"], "dst": ["autogroup:member"], "ip": ["*"] }
      ]

EOF
	if tailscale status --json --peers=false 2>/dev/null | grep -q '"BackendState": *"Running"'; then
		note "This machine is on the tailnet already."
	else
		local up=(tailscale up --advertise-tags=tag:nodus)
		if [ -n "${TS_AUTHKEY:-}" ]; then up+=(--auth-key="$TS_AUTHKEY"); fi
		note "Joining as tag:nodus. Open the login link Tailscale prints:"
		if ! "${up[@]}" 2>&1 | tee -a "$LOG"; then
			warn "Tailscale refused tag:nodus (nobody owns the tag yet?). Joining untagged: add the policy above, then run: sudo tailscale up --advertise-tags=tag:nodus"
			up=(tailscale up)
			if [ -n "${TS_AUTHKEY:-}" ]; then up+=(--auth-key="$TS_AUTHKEY"); fi
			"${up[@]}" 2>&1 | tee -a "$LOG" || fail "tailscale up failed."
		fi
	fi
	if ! tailscale status --json --peers=false 2>/dev/null | grep -q '"tag:nodus"'; then
		warn "This server isn't tagged tag:nodus, so the tailnet's rules for it don't apply. Add the policy above, then run: sudo tailscale up --advertise-tags=tag:nodus"
	fi
	# serve picks its address when it starts: now the Tailscale one.
	run systemctl restart nodus.service
	wait_until_up
}

# --------------------------------------------------------------------------
# 10. The address and a setup code

finish() {
	local address url code="" out
	address=$(listening_on || true)
	if [ "${address%:*}" = "127.0.0.1" ]; then
		url="http://127.0.0.1:$PORT"
	else
		local name
		name=$(tailscale status --json --peers=false 2>/dev/null |
			sed -n 's/.*"DNSName": *"\([^".]*\)[^"]*".*/\1/p' | sed -n 1p || true)
		url="http://${name:-${address%:*}}:$PORT"
	fi

	# `nodus setup-code` comes with sign-in (SRV-060); older servers don't have it.
	if out=$(as_user "$SERVE_USER" "$BIN" setup-code 2>&1); then
		code=$(printf '%s\n' "$out" | grep . | tail -n1)
	elif printf '%s' "$out" | grep -q "^Usage:"; then
		code=""
		out="This server version has no setup codes yet."
	fi

	printf '\nNodus server is running.\n\n'
	if [ -n "$code" ]; then
		printf '  In Nodus: Connect to server… → %s → code %s\n' "$url" "$code"
	else
		printf '  In Nodus: Connect to server… → %s\n' "$url"
		printf '  (%s)\n' "$(printf '%s\n' "$out" | sed -n 1p)"
	fi
	if [ "${address%:*}" = "127.0.0.1" ]; then
		printf '\n  It listens on this machine only, until it joins Tailscale (--tailscale).\n'
		printf '  From your computer, tunnel to it first: ssh -N -L %s:127.0.0.1:%s <you>@<this server>\n' "$PORT" "$PORT"
	fi
	printf '\n  Logs: journalctl -u nodus -u nodus-host    Install log: %s\n' "$LOG"
	printf '  Give this VM no cloud instance role, and add your provider'"'"'s firewall if it has one.\n\n'
}

# --------------------------------------------------------------------------
# --uninstall [--purge] (SRV-012)

uninstall() {
	STEP="Uninstalling"
	say "Stopping and removing nodus.service and nodus-host.service"
	local unit
	for unit in nodus.service nodus-host.service; do
		if [ -f "$UNIT_DIR/$unit" ]; then
			run systemctl disable --now "$unit" || true
			rm -f "$UNIT_DIR/$unit"
		fi
	done
	# KillMode=process leaves the host's sessions behind when it stops: they
	# are still in its cgroup.
	local procs=/sys/fs/cgroup/system.slice/nodus-host.service/cgroup.procs pid
	if [ -r "$procs" ]; then
		while read -r pid; do
			kill -KILL "$pid" 2>/dev/null || true
		done <"$procs"
	fi
	run systemctl daemon-reload
	run systemctl reset-failed nodus.service nodus-host.service || true
	say "Removing $BIN"
	rm -f "$BIN" "$BIN.new"
	rmdir "$BIN_DIR" /opt/nodus 2>/dev/null || true

	if [ "$PURGE" = 0 ]; then
		printf '\nNodus server removed. Kept: the data (%s, %s, %s), the users %s and %s,\n' \
			"$SERVE_HOME" "$RUN_HOME" /srv/nodus "$SERVE_USER" "$RUN_USER"
		printf '%s'"'"'s rootless Docker, and the firewall (nodus-firewall.service), so nothing\n' "$RUN_USER"
		printf 'left running is exposed. Installing again picks the data up; --uninstall --purge removes it all.\n'
		return
	fi

	say "Purging: $RUN_USER's Docker, both users and their data, and the firewall"
	if getent passwd "$RUN_USER" >/dev/null; then
		local uid
		uid=$(id -u "$RUN_USER")
		if [ -d "/run/user/$uid" ]; then
			run as_user "$RUN_USER" systemctl --user disable --now docker.service || true
		fi
		run loginctl disable-linger "$RUN_USER" || true
		run loginctl terminate-user "$RUN_USER" || true
		run systemctl stop "user@$uid.service" || true
	fi
	local user
	for user in "$SERVE_USER" "$RUN_USER"; do
		if getent passwd "$user" >/dev/null; then
			run pkill -KILL -u "$user" || true
			sleep 1
			run userdel --remove "$user" || true
		fi
		if getent group "$user" >/dev/null; then
			run groupdel "$user" || true
		fi
		sed -i "/^$user:/d" /etc/subuid /etc/subgid
	done
	rm -rf "$SERVE_HOME" "$RUN_HOME" /srv/nodus
	if [ -f "$UNIT_DIR/nodus-firewall.service" ]; then
		run systemctl disable --now nodus-firewall.service || true
		rm -f "$UNIT_DIR/nodus-firewall.service"
		run systemctl daemon-reload
	fi
	nft delete table inet nodus 2>/dev/null || true
	rm -rf /etc/nodus
	local profile
	for profile in /etc/apparmor.d/*rootlesskit; do
		if [ -f "$profile" ] && grep -q "Written by Nodus's install-server.sh" "$profile"; then
			run apparmor_parser -R "$profile" || true
			rm -f "$profile"
		fi
	done
	printf '\nNodus server purged. The packages it installed (git, gh, Docker, nftables, Tailscale) stay.\n'
}

# All of the script is read before any of it runs, so `curl | bash` can't
# run half a download, and nothing it runs can eat the rest from stdin.
main "$@" </dev/null
exit
