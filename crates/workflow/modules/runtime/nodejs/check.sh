# Read-only. Up to date when the NodeSource repository for the chosen major is set up and
# the installed Node.js has that major version.
export LC_ALL=C
key=/etc/apt/keyrings/nodesource.gpg
src=/etc/apt/sources.list.d/nodesource.sources
render() {
  echo "Types: deb"
  echo "URIs: https://deb.nodesource.com/node_${SV_MAJOR}.x/"
  echo "Suites: nodistro"
  echo "Components: main"
  echo "Signed-By: $key"
}
if [ ! -s "$key" ]; then
  echo "The NodeSource signing key is not in place yet."
  exit 10
fi
if [ "$(cat "$src" 2>/dev/null)" != "$(render)" ]; then
  echo "The NodeSource apt repository is not set up for Node.js $SV_MAJOR."
  exit 10
fi
ver=$(node --version 2>/dev/null || true)
case "$ver" in
  v"$SV_MAJOR".*) echo "Node.js $ver is installed." ;;
  *) echo "Node.js $SV_MAJOR is not installed (found: ${ver:-none})."; exit 10 ;;
esac
