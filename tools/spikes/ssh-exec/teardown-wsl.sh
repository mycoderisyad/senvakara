#!/usr/bin/env bash
# Undo setup-wsl.sh. Run as root inside WSL Ubuntu-24.04:
#   wsl -d Ubuntu-24.04 -u root -- bash /mnt/d/LAIN-LAIN/code/senvakara/tools/spikes/ssh-exec/teardown-wsl.sh
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run as root (wsl -u root)" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"

pkill -u svtest >/dev/null 2>&1 || true
userdel --remove svtest >/dev/null 2>&1 || true
rm -f /etc/sudoers.d/90-senvakara-spike /etc/ssh/sshd_config.d/10-senvakara-spike.conf
systemctl disable --now ssh.service >/dev/null 2>&1 || true
apt-get purge -y -qq openssh-server >/dev/null
rm -f "${here}/creds.local"
echo "ok: spike host removed"
