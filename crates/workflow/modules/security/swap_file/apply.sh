# Create (or resize) /swapfile, turn it on, list it in /etc/fstab.
export LC_ALL=C
file=/swapfile
want=$((SV_SIZE_MB * 1024 * 1024))

if [ -e "$file" ] && [ ! -f "$file" ]; then
  echo "$file exists but is not a regular file; nothing was changed." >&2
  exit 3
fi
if [ -f "$file" ] && [ "$(stat -c %s "$file")" -ne "$want" ]; then
  swapoff "$file" 2>/dev/null || true
  rm -f "$file"
  echo "Removed the old $file to make it $SV_SIZE_MB MB."
fi

if [ ! -f "$file" ]; then
  if [ "$(stat -f -c %T /)" = btrfs ]; then
    echo "/ is btrfs, which needs a swap file made for it (btrfs filesystem mkswapfile); nothing was changed." >&2
    exit 3
  fi
  free=$(df --output=avail -B1 / | tail -n 1)
  if [ "$free" -lt $((want + 1024 * 1024 * 1024)) ]; then
    echo "Not enough free disk: $SV_SIZE_MB MB plus 1 GB to spare is needed." >&2
    exit 3
  fi
  (umask 077 && fallocate -l "$want" "$file") || {
    rm -f "$file"
    (umask 077 && dd if=/dev/zero of="$file" bs=1M count="$SV_SIZE_MB" status=none)
  }
  chmod 0600 "$file"
  mkswap "$file" >/dev/null
  echo "Created $file ($SV_SIZE_MB MB)."
fi

if ! swapon --show=NAME --noheadings | grep -qx "$file"; then
  swapon "$file"
fi
if ! grep -qE "^$file[[:space:]]+none[[:space:]]+swap[[:space:]]" /etc/fstab; then
  printf '%s none swap sw 0 0\n' "$file" >>/etc/fstab
fi
swapon --show
