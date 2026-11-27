# Refresh the package lists, then install the updates. Config files the
# admin changed are kept (--force-confold); new defaults land as .dpkg-dist.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
# Wait for a running apt or unattended-upgrades instead of failing at once.
lock=(-o DPkg::Lock::Timeout=300)

apt-get -q "${lock[@]}" update
opts=(-y -q "${lock[@]}" -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold)
if [ "$SV_FULL_UPGRADE" = 1 ]; then
  apt-get "${opts[@]}" dist-upgrade
else
  apt-get "${opts[@]}" upgrade
fi

install -d -m 0755 /var/lib/senvakara
touch /var/lib/senvakara/apt-updated

if [ -e /run/reboot-required ]; then
  echo "The host asks for a reboot to finish these updates."
fi
