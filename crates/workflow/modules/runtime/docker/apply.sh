# Add Docker's apt repository, install Engine, Buildx and Compose, start the service.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "Docker's repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install ca-certificates curl

key=/etc/apt/keyrings/docker.asc
src=/etc/apt/sources.list.d/docker.sources
install -m 0755 -d /etc/apt/keyrings
if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o "$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

if [ "$ID" = ubuntu ]; then suite=${UBUNTU_CODENAME:-$VERSION_CODENAME}; else suite=$VERSION_CODENAME; fi
tmp=$(mktemp)
{
  echo "Types: deb"
  echo "URIs: https://download.docker.com/linux/$ID"
  echo "Suites: $suite"
  echo "Components: stable"
  echo "Architectures: $(dpkg --print-architecture)"
  echo "Signed-By: $key"
} >"$tmp"
if [ "$(cat "$src" 2>/dev/null)" != "$(cat "$tmp")" ]; then
  install -m 0644 "$tmp" "$src"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
systemctl enable --now docker

if [ "${SV_ADD_USER:-0}" = 1 ] && [ -n "${SV_SESSION_USER:-}" ] && [ "$SV_SESSION_USER" != root ]; then
  usermod -aG docker "$SV_SESSION_USER"
  echo "Added $SV_SESSION_USER to the docker group; the change applies at the next sign-in."
fi
docker compose version
