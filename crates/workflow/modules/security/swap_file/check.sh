# Read-only. Up to date when /swapfile has the size asked for, is in use and
# is listed in /etc/fstab.
export LC_ALL=C
file=/swapfile
want=$((SV_SIZE_MB * 1024 * 1024))

if [ ! -f "$file" ]; then
  echo "$file does not exist."
  exit 10
fi
if [ "$(stat -c %s "$file")" -ne "$want" ]; then
  echo "$file is $(( $(stat -c %s "$file") / 1024 / 1024 )) MB, not $SV_SIZE_MB MB."
  exit 10
fi
if ! swapon --show=NAME --noheadings | grep -qx "$file"; then
  echo "$file is not in use."
  exit 10
fi
if ! grep -qE "^$file[[:space:]]+none[[:space:]]+swap[[:space:]]" /etc/fstab; then
  echo "$file is not in /etc/fstab, so it would be off after a reboot."
  exit 10
fi
echo "$file ($SV_SIZE_MB MB) is in use."
