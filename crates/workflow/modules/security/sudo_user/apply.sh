# Create the user (if needed), add it to sudo, add the key, and write or
# remove the passwordless sudoers drop-in. Nothing that is there already is
# removed: other keys in authorized_keys stay.
export LC_ALL=C
user=$SV_USERNAME
if [ "$user" = root ]; then
  echo "Choose a user other than root." >&2
  exit 2
fi

# The key was checked in Senvakara; ssh-keygen checks it once more.
tmp=$(mktemp)
printf '%s\n' "$SV_PUBLIC_KEY" >"$tmp"
if ! ssh-keygen -l -f "$tmp" >/dev/null 2>&1; then
  rm -f "$tmp"
  echo "ssh-keygen does not accept the public key." >&2
  exit 2
fi
rm -f "$tmp"

if ! id "$user" >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash "$user"
  echo "Created user $user."
fi
usermod -aG sudo "$user"

home=$(getent passwd "$user" | cut -d: -f6)
group=$(id -gn "$user")
# Root writes into the user's home: never follow a link planted there.
for path in "$home/.ssh" "$home/.ssh/authorized_keys"; do
  if [ -L "$path" ]; then
    echo "$path is a symbolic link; fix it by hand first." >&2
    exit 2
  fi
done
install -d -m 0700 -o "$user" -g "$group" "$home/.ssh"
keys="$home/.ssh/authorized_keys"
[ -e "$keys" ] || install -m 0600 -o "$user" -g "$group" /dev/null "$keys"
key=$(printf '%s' "$SV_PUBLIC_KEY" | awk '{ print $1 " " $2 }')
if ! grep -qF -- "$key" "$keys"; then
  printf '%s\n' "$SV_PUBLIC_KEY" >>"$keys"
  echo "Added the key to $keys."
fi
chown "$user:$group" "$home/.ssh" "$keys"
chmod 0700 "$home/.ssh"
chmod 0600 "$keys"

sudoers="/etc/sudoers.d/90-senvakara-$user"
if [ "$SV_PASSWORDLESS_SUDO" = 1 ]; then
  tmp=$(mktemp)
  printf '%s ALL=(ALL:ALL) NOPASSWD: ALL\n' "$user" >"$tmp"
  visudo -cf "$tmp" >/dev/null
  install -m 0440 "$tmp" "$sudoers"
  rm -f "$tmp"
  echo "$user may use sudo without a password."
else
  rm -f "$sudoers"
  if passwd -S "$user" | awk '{ exit !($2 == "L" || $2 == "NP") }'; then
    echo "Note: $user has no password yet, so sudo cannot be used as $user until one is set (sudo passwd $user)."
  fi
fi
