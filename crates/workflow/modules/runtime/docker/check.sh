# Read-only. Up to date when Docker's repository is set up, the engine packages are
# installed, the service runs and `docker compose` answers.
export LC_ALL=C
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "Docker's repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac

key=/etc/apt/keyrings/docker.asc
src=/etc/apt/sources.list.d/docker.sources
render() {
  if [ "$ID" = ubuntu ]; then suite=${UBUNTU_CODENAME:-$VERSION_CODENAME}; else suite=$VERSION_CODENAME; fi
  echo "Types: deb"
  echo "URIs: https://download.docker.com/linux/$ID"
  echo "Suites: $suite"
  echo "Components: stable"
  echo "Architectures: $(dpkg --print-architecture)"
  echo "Signed-By: $key"
}

if [ ! -s "$key" ]; then
  echo "Docker's signing key is not in place yet."
  exit 10
fi
if [ "$(cat "$src" 2>/dev/null)" != "$(render)" ]; then
  echo "Docker's apt repository is not set up as expected."
  exit 10
fi
for p in docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin; do
  if ! dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed'; then
    echo "The package $p is not installed."
    exit 10
  fi
done
if ! systemctl is-active --quiet docker; then
  echo "The docker service is not running."
  exit 10
fi
if ! docker compose version >/dev/null 2>&1; then
  echo "docker compose does not answer."
  exit 10
fi
if [ "${SV_ADD_USER:-0}" = 1 ] && [ -n "${SV_SESSION_USER:-}" ] && [ "$SV_SESSION_USER" != root ]; then
  if ! id -nG "$SV_SESSION_USER" 2>/dev/null | tr ' ' '\n' | grep -qx docker; then
    echo "$SV_SESSION_USER is not in the docker group yet."
    exit 10
  fi
fi
echo "Docker Engine runs and docker compose is available."
