# Add the NodeSource apt repository for the chosen major and install the nodejs package.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install ca-certificates curl gnupg

key=/etc/apt/keyrings/nodesource.gpg
src=/etc/apt/sources.list.d/nodesource.sources
install -m 0755 -d /etc/apt/keyrings
if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor >"$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
{
  echo "Types: deb"
  echo "URIs: https://deb.nodesource.com/node_${SV_MAJOR}.x/"
  echo "Suites: nodistro"
  echo "Components: main"
  echo "Signed-By: $key"
} >"$tmp"
if [ "$(cat "$src" 2>/dev/null)" != "$(cat "$tmp")" ]; then
  install -m 0644 "$tmp" "$src"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install nodejs
# Switching to a lower major: apt keeps the newer installed package unless given the exact version.
case "$(node --version 2>/dev/null || true)" in
  v"$SV_MAJOR".*) ;;
  *)
    ver=$(apt-cache madison nodejs | awk -v m="$SV_MAJOR" '!found && $3 ~ ("^" m "[.]") { print $3; found = 1 }')
    if [ -z "$ver" ]; then
      echo "The NodeSource repository offers no Node.js $SV_MAJOR package for this host."
      exit 1
    fi
    apt-get -y -q "${lock[@]}" install --allow-downgrades "nodejs=$ver"
    ;;
esac
node --version
