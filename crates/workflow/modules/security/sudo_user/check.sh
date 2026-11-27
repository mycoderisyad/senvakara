# Read-only. Up to date when the user exists, is in the sudo group, has the
# key in a private authorized_keys, and the sudoers drop-in matches the
# passwordless setting.
export LC_ALL=C
user=$SV_USERNAME
if [ "$user" = root ]; then
  echo "Choose a user other than root." >&2
  exit 2
fi
if ! id "$user" >/dev/null 2>&1; then
  echo "User $user does not exist yet."
  exit 10
fi
if ! id -nG "$user" | tr ' ' '\n' | grep -qx sudo; then
  echo "$user is not in the sudo group."
  exit 10
fi

home=$(getent passwd "$user" | cut -d: -f6)
keys="$home/.ssh/authorized_keys"
key=$(printf '%s' "$SV_PUBLIC_KEY" | awk '{ print $1 " " $2 }')
if [ ! -f "$keys" ] || [ -L "$keys" ] || ! grep -qF -- "$key" "$keys"; then
  echo "The key is not in $keys yet."
  exit 10
fi
if [ "$(stat -c '%a %U' "$home/.ssh")" != "700 $user" ] || [ "$(stat -c '%a %U' "$keys")" != "600 $user" ]; then
  echo "$home/.ssh is not private to $user."
  exit 10
fi

sudoers="/etc/sudoers.d/90-senvakara-$user"
if [ "$SV_PASSWORDLESS_SUDO" = 1 ]; then
  if [ "$(cat "$sudoers" 2>/dev/null)" != "$user ALL=(ALL:ALL) NOPASSWD: ALL" ]; then
    echo "Passwordless sudo is not set up for $user."
    exit 10
  fi
elif [ -e "$sudoers" ]; then
  echo "$sudoers still allows sudo without a password."
  exit 10
fi
echo "$user can sign in with the key and use sudo."
