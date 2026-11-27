# Install python3, python3-venv and python3-pip from the distribution (and pipx if asked).
# python.org publishes no apt repository, so the distribution Python is used.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
pkgs=(python3 python3-venv python3-pip)
[ "${SV_PIPX:-0}" = 1 ] && pkgs+=(pipx)
apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install "${pkgs[@]}"
python3 --version
