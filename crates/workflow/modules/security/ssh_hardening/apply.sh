# Write the drop-in, test it with sshd -t, then reload sshd without ending
# open sessions. Senvakara scheduled rollback.sh before this ran; the
# backup it restores is written to $SV_GUARD_DIR first.
export LC_ALL=C
conf=/etc/ssh/sshd_config.d/00-senvakara-hardening.conf
main=/etc/ssh/sshd_config

# Anti-lockout (plan WF §4): never take away the way this session signs in.
if [ "$SV_DISABLE_PASSWORDS" = 1 ] && [ "$SV_SESSION_AUTH" != key ]; then
  echo "Senvakara signs in to this host with a password, so turning passwords off would lock it out. Switch the host to a key in Hosts, then run again." >&2
  exit 3
fi
if [ "$SV_PERMIT_ROOT_LOGIN" = no ] && [ "$SV_SESSION_USER" = root ]; then
  echo "Senvakara signs in to this host as root, so turning root sign-in off would lock it out. Switch the host in Hosts to your sudo user, then run again." >&2
  exit 3
fi
if ! grep -qiE '^[[:space:]]*Include[[:space:]]+/etc/ssh/sshd_config\.d/\*\.conf' "$main"; then
  echo "$main does not include /etc/ssh/sshd_config.d/*.conf, so a drop-in would not be read." >&2
  exit 3
fi

# Backup for rollback.sh.
install -d -m 0700 "$SV_GUARD_DIR"
cp -a "$main" "$SV_GUARD_DIR/sshd_config"
if [ -f "$conf" ]; then
  cp -a "$conf" "$SV_GUARD_DIR/drop-in"
else
  touch "$SV_GUARD_DIR/drop-in.absent"
fi

restore() {
  cp -a "$SV_GUARD_DIR/sshd_config" "$main"
  if [ -f "$SV_GUARD_DIR/drop-in" ]; then
    cp -a "$SV_GUARD_DIR/drop-in" "$conf"
  else
    rm -f "$conf"
  fi
}

tmp=$(mktemp)
{
  echo "# Written by Senvakara (SSH hardening node). Change it there."
  [ -n "${SV_PORT:-}" ] && echo "Port $SV_PORT"
  echo "PermitRootLogin $SV_PERMIT_ROOT_LOGIN"
  if [ "$SV_DISABLE_PASSWORDS" = 1 ]; then
    echo "PasswordAuthentication no"
    echo "KbdInteractiveAuthentication no"
  fi
  echo "PermitEmptyPasswords no"
} >"$tmp"
install -m 0644 "$tmp" "$conf"
rm -f "$tmp"

if [ -n "${SV_PORT:-}" ]; then
  # Port lines add up; the drop-in's port replaces the main file's.
  sed -i -E 's/^[[:space:]]*[Pp][Oo][Rr][Tt][[:space:]]+/# Moved to sshd_config.d by Senvakara: &/' "$main"
  if command -v ufw >/dev/null 2>&1 && ufw status | grep -q '^Status: active'; then
    ufw allow "$SV_PORT/tcp" comment 'SSH (Senvakara)'
    echo "$SV_PORT/tcp" >"$SV_GUARD_DIR/ufw-added"
  fi
fi

install -d -m 0755 /run/sshd
if ! sshd -t; then
  restore
  echo "sshd -t refused the new settings; the previous files are back." >&2
  exit 4
fi

# Ubuntu 22.10+ may start sshd through ssh.socket, which only picks up a new
# port after a daemon-reload and a socket restart. Open sessions stay.
if [ -n "${SV_PORT:-}" ] && systemctl is-active --quiet ssh.socket; then
  systemctl daemon-reload
  systemctl restart ssh.socket
fi
systemctl try-reload-or-restart ssh
echo "sshd reloaded with the new settings."
