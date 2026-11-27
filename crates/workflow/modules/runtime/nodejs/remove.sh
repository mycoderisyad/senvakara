# Purge the nodejs package and remove the NodeSource repository and key.
# Global npm packages and project folders are not touched.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
apt-get -y -q "${lock[@]}" purge nodejs 2>/dev/null || true
apt-get -y -q "${lock[@]}" autoremove
rm -f /etc/apt/sources.list.d/nodesource.sources /etc/apt/keyrings/nodesource.gpg
apt-get -q "${lock[@]}" update || true
echo "Node.js is removed. Projects and per-user npm folders such as ~/.npm were left in place."
