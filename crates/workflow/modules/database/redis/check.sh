# Read-only. Up to date when the Redis repository is set up, the package is installed
# and redis-server runs.
export LC_ALL=C
. /etc/os-release
key=/etc/apt/keyrings/redis.gpg
src=/etc/apt/sources.list.d/redis.sources
render() {
  echo "Types: deb"
  echo "URIs: https://packages.redis.io/deb"
  echo "Suites: $VERSION_CODENAME"
  echo "Components: main"
  echo "Signed-By: $key"
}
if [ ! -s "$key" ]; then
  echo "The Redis signing key is not in place yet."
  exit 10
fi
if [ "$(cat "$src" 2>/dev/null)" != "$(render)" ]; then
  echo "The Redis apt repository is not set up as expected."
  exit 10
fi
if ! dpkg-query -W -f='${Status}' redis 2>/dev/null | grep -q 'install ok installed'; then
  echo "The redis package is not installed."
  exit 10
fi
if ! systemctl is-active --quiet redis-server; then
  echo "redis-server is not running."
  exit 10
fi
echo "Redis runs from the official repository."
