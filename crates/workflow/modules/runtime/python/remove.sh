# Remove only the extras apply.sh added (python3-venv, python3-pip, pipx).
# python3 itself stays: the operating system depends on it.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
apt-get -y -q "${lock[@]}" remove python3-venv python3-pip pipx 2>/dev/null || true
echo "python3-venv, python3-pip and pipx are removed. python3 was kept because the system needs it; virtual environments and pip-installed files were left in place."
