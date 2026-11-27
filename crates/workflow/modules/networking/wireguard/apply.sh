# Install WireGuard, generate the server key once, write wg0.conf (0600), enable
# wg-quick@wg0 and print the server public key for the client.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
port="${SV_LISTEN_PORT:-51820}"
addr="${SV_ADDRESS:-}"
peer="${SV_PEER_PUBLIC_KEY:-}"
allowed="${SV_PEER_ALLOWED_IPS:-}"
lock=(-o DPkg::Lock::Timeout=300)
dir=/etc/wireguard
keyfile=$dir/senvakara-server.key
conf=$dir/wg0.conf

valid_cidr() {
  local ip=${1%/*} len=${1#*/} o
  [[ "$1" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}/[0-9]{1,2}$ ]] || return 1
  [ "$len" -le 32 ] || return 1
  IFS=. read -ra oct <<<"$ip"
  for o in "${oct[@]}"; do [ "$((10#$o))" -le 255 ] || return 1; done
}
valid_cidr "$addr" || { echo "The address must look like 10.8.0.1/24."; exit 2; }
[[ "$port" =~ ^[0-9]{1,5}$ ]] && [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || { echo "The listen port must be 1-65535."; exit 2; }
if [ -n "$peer" ]; then
  [[ "$peer" =~ ^[A-Za-z0-9+/]{43}=$ ]] || { echo "The peer public key must be 44 base64 characters."; exit 2; }
  [ -n "$allowed" ] || { echo "A peer needs its allowed IPs."; exit 2; }
  IFS=, read -ra list <<<"${allowed// /}"
  for c in "${list[@]}"; do valid_cidr "$c" || { echo "Allowed IP '$c' is not a valid CIDR."; exit 2; }; done
fi

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install wireguard

install -d -m 0700 "$dir"
if [ ! -s "$keyfile" ]; then
  (umask 077; wg genkey >"$keyfile")
fi
chmod 0600 "$keyfile"
key=$(cat "$keyfile")

tmp=$(umask 077; mktemp)
{
  printf '# Written by Senvakara (WireGuard node). Change it there.\n'
  printf '[Interface]\nAddress = %s\nListenPort = %s\nPrivateKey = %s\n' "$addr" "$port" "$key"
  if [ -n "$peer" ]; then
    printf '\n[Peer]\nPublicKey = %s\nAllowedIPs = %s\n' "$peer" "${allowed// /}"
  fi
} >"$tmp"
changed=0
if ! cmp -s "$tmp" "$conf"; then
  install -m 0600 -o root -g root "$tmp" "$conf"
  changed=1
fi
chmod 0600 "$conf"
rm -f "$tmp"

systemctl enable wg-quick@wg0
if [ "$changed" = 1 ] && systemctl is-active --quiet wg-quick@wg0; then
  systemctl restart wg-quick@wg0
elif ! systemctl is-active --quiet wg-quick@wg0; then
  systemctl start wg-quick@wg0
fi

echo "WireGuard wg0 is up on UDP port $port. Allow it in the firewall node."
echo "Server public key (put this in the client's [Peer] PublicKey): $(wg pubkey <"$keyfile")"
