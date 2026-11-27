# Read-only. Up to date when python3, python3-venv and python3-pip are installed
# (and pipx when asked for) and python3 runs.
export LC_ALL=C
pkgs="python3 python3-venv python3-pip"
[ "${SV_PIPX:-0}" = 1 ] && pkgs="$pkgs pipx"
for p in $pkgs; do
  if ! dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed'; then
    echo "The package $p is not installed."
    exit 10
  fi
done
if ! ver=$(python3 --version 2>&1); then
  echo "python3 does not run."
  exit 10
fi
echo "$ver with venv and pip is installed."
