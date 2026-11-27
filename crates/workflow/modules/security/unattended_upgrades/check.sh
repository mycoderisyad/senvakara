# Read-only. Up to date when unattended-upgrades is installed, apt's
# effective settings switch the daily list refresh and the upgrade run on,
# and the timer that starts them is enabled.
export LC_ALL=C

if ! dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null | grep -q 'install ok installed'; then
  echo "unattended-upgrades is not installed."
  exit 10
fi

setting() {
  apt-config dump "$1" | sed -n "s/^$1 \"\(.*\)\";\$/\1/p" | tail -n 1
}
lists=$(setting APT::Periodic::Update-Package-Lists)
upgrade=$(setting APT::Periodic::Unattended-Upgrade)
echo "SV_FACT update_package_lists=${lists:-unset}"
echo "SV_FACT unattended_upgrade=${upgrade:-unset}"
if [ -z "$lists" ] || [ "$lists" = 0 ] || [ -z "$upgrade" ] || [ "$upgrade" = 0 ]; then
  echo "Automatic updates are not switched on in apt's settings."
  exit 10
fi

if ! systemctl is-enabled --quiet apt-daily-upgrade.timer 2>/dev/null; then
  echo "apt-daily-upgrade.timer is not enabled."
  exit 10
fi
echo "Automatic security updates are on."
