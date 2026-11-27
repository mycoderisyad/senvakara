# Run by the host itself (a systemd timer) when Senvakara could not sign in
# again after the change: put the previous sshd files back and reload.
# Best effort: one failed step must not stop the others.
set +e
export LC_ALL=C
conf=/etc/ssh/sshd_config.d/00-senvakara-hardening.conf

[ -f "$SV_GUARD_DIR/sshd_config" ] && cp -a "$SV_GUARD_DIR/sshd_config" /etc/ssh/sshd_config
if [ -f "$SV_GUARD_DIR/drop-in" ]; then
  cp -a "$SV_GUARD_DIR/drop-in" "$conf"
elif [ -e "$SV_GUARD_DIR/drop-in.absent" ]; then
  rm -f "$conf"
fi
if [ -f "$SV_GUARD_DIR/ufw-added" ]; then
  ufw delete allow "$(cat "$SV_GUARD_DIR/ufw-added")"
fi

install -d -m 0755 /run/sshd
if systemctl is-active --quiet ssh.socket; then
  systemctl daemon-reload
  systemctl restart ssh.socket
fi
systemctl try-reload-or-restart ssh
logger -t senvakara "SSH settings rolled back: Senvakara did not sign in again in time"
