# Run by the host itself (a systemd timer) when Senvakara could not sign in
# again after the change: put ufw back the way it was.
# Best effort: one failed step must not stop the others.
set +e
export LC_ALL=C

# apply.sh stopped before it changed ufw: there is nothing to undo.
[ -f "$SV_GUARD_DIR/was" ] || exit 0
if [ "$(cat "$SV_GUARD_DIR/was")" != active ]; then
  ufw --force disable
  systemctl stop ufw
fi
if [ -d "$SV_GUARD_DIR/etc-ufw" ] && [ -f "$SV_GUARD_DIR/default-ufw" ]; then
  rm -rf /etc/ufw
  cp -a "$SV_GUARD_DIR/etc-ufw" /etc/ufw
  cp -a "$SV_GUARD_DIR/default-ufw" /etc/default/ufw
fi
if [ "$(cat "$SV_GUARD_DIR/was" 2>/dev/null)" = active ]; then
  ufw reload
fi
logger -t senvakara "Firewall rolled back: Senvakara did not sign in again in time"
