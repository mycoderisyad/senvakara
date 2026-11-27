# Install unattended-upgrades and switch it on with the two APT::Periodic
# settings its documentation names. The distribution's own
# 50unattended-upgrades (which origins are upgraded) is left as shipped.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)

if ! dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null | grep -q 'install ok installed'; then
  apt-get -q "${lock[@]}" update
  apt-get -y -q "${lock[@]}" install unattended-upgrades
fi

conf=/etc/apt/apt.conf.d/20auto-upgrades
tmp=$(mktemp)
printf '%s\n' \
  'APT::Periodic::Update-Package-Lists "1";' \
  'APT::Periodic::Unattended-Upgrade "1";' >"$tmp"
install -m 0644 "$tmp" "$conf"
rm -f "$tmp"

systemctl enable --now apt-daily.timer apt-daily-upgrade.timer
echo "Wrote $conf; the daily timers are enabled."
