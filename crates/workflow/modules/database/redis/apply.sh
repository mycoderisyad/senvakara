# Add the official Redis apt repository, install the redis package, enable redis-server.
# The packaged configuration binds to localhost only; this script does not change it.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
. /etc/os-release

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install ca-certificates curl gpg

key=/etc/apt/keyrings/redis.gpg
src=/etc/apt/sources.list.d/redis.sources
install -m 0755 -d /etc/apt/keyrings
if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://packages.redis.io/gpg | gpg --dearmor >"$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
{
  echo "Types: deb"
  echo "URIs: https://packages.redis.io/deb"
  echo "Suites: $VERSION_CODENAME"
  echo "Components: main"
  echo "Signed-By: $key"
} >"$tmp"
if [ "$(cat "$src" 2>/dev/null)" != "$(cat "$tmp")" ]; then
  install -m 0644 "$tmp" "$src"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install redis
systemctl enable --now redis-server
echo "Redis is installed and runs as redis-server."
