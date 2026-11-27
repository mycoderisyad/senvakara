# Stop Docker, purge its packages and remove Docker's repository and key.
# Images, containers and volumes are kept in /var/lib/docker and /var/lib/containerd.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now docker.service docker.socket containerd.service 2>/dev/null || true
apt-get -y -q "${lock[@]}" purge docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin 2>/dev/null || true
apt-get -y -q "${lock[@]}" autoremove
rm -f /etc/apt/sources.list.d/docker.sources /etc/apt/keyrings/docker.asc
apt-get -q "${lock[@]}" update || true
echo "Docker is removed. Images, containers and volumes were left in /var/lib/docker and /var/lib/containerd."
