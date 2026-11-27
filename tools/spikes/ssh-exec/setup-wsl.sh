#!/usr/bin/env bash
# Phase 0 spike (WF) test host. Run as root inside WSL Ubuntu-24.04, e.g. from
# PowerShell:  wsl -d Ubuntu-24.04 -u root -- bash /mnt/d/LAIN-LAIN/code/senvakara/tools/spikes/ssh-exec/setup-wsl.sh
#
# - installs openssh-server, listening on 127.0.0.1:2222 only
# - creates the throwaway user `svtest` with a random password and
#   password-required sudo (so `sudo -S` is really exercised)
# - writes the test credentials to creds.local next to this script
#   (gitignored); nothing is printed
# Undo with teardown-wsl.sh.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "run as root (wsl -u root)" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq openssh-server >/dev/null

cat >/etc/ssh/sshd_config.d/10-senvakara-spike.conf <<'EOF'
# Senvakara Phase 0 spike: loopback only, test port.
Port 2222
ListenAddress 127.0.0.1
PasswordAuthentication yes
EOF
# `sshd -t` needs the privilege separation dir that the service creates.
install -d -m 0755 /run/sshd
sshd -t

# Ubuntu 24.04 starts sshd through ssh.socket on port 22; use the plain
# service so the Port/ListenAddress above are read directly.
systemctl disable --now ssh.socket >/dev/null 2>&1 || true
systemctl enable ssh.service >/dev/null 2>&1
systemctl restart ssh.service

if ! id svtest >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash svtest
fi
pw="$(head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 24)"
echo "svtest:${pw}" | chpasswd

echo 'svtest ALL=(ALL:ALL) ALL' >/etc/sudoers.d/90-senvakara-spike
chmod 0440 /etc/sudoers.d/90-senvakara-spike
visudo -cf /etc/sudoers.d/90-senvakara-spike >/dev/null

umask 077
cat >"${here}/creds.local" <<EOF
host=127.0.0.1
port=2222
user=svtest
password=${pw}
EOF

ss -ltn 'sport = :2222' | tail -n +2
echo "ok: sshd on 127.0.0.1:2222, user svtest, credentials in creds.local"
