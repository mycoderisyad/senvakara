# Read-only. Up to date when WireGuard is installed, the server key exists and wg0.conf
# matches the settings, and wg-quick@wg0 is enabled and running. Never prints the private key.
export LC_ALL=C
port="${SV_LISTEN_PORT:-51820}"
addr="${SV_ADDRESS:-}"
peer="${SV_PEER_PUBLIC_KEY:-}"
allowed="${SV_PEER_ALLOWED_IPS:-}"
dir=/etc/wireguard
conf=$dir/wg0.conf

valid_cidr() {
  local ip=${1%/*} len=${1#*/} o
  [[ "$1" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}/[0-9]{1,2}$ ]] || return 1
  [ "$len" -le 32 ] || return 1
  IFS=. read -ra oct <<<"$ip"
  for o in "${oct[@]}"; do [ "$((10#$o))" -le 255 ] || return 1; done
}
valid_cidr "$addr" || { echo "The address must look like 10.8.0.1/24."; exit 2; }
if [ -n "$peer" ]; then
  [[ "$peer" =~ ^[A-Za-z0-9+/]{43}=$ ]] || { echo "The peer public key must be 44 base64 characters."; exit 2; }
  [ -n "$allowed" ] || { echo "A peer needs its allowed IPs."; exit 2; }
  IFS=, read -ra list <<<"${allowed// /}"
  for c in "${list[@]}"; do valid_cidr "$c" || { echo "Allowed IP '$c' is not a valid CIDR."; exit 2; }; done
fi

if ! dpkg-query -W -f='${Status}' wireguard-tools 2>/dev/null | grep -q 'install ok installed'; then
  echo "WireGuard is not installed."; exit 10
fi
if [ ! -s "$dir/senvakara-server.key" ]; then
  echo "The server key has not been generated yet."; exit 10
fi
key=$(cat "$dir/senvakara-server.key")
render() {
  printf '# Written by Senvakara (WireGuard node). Change it there.\n'
  printf '[Interface]\nAddress = %s\nListenPort = %s\nPrivateKey = %s\n' "$addr" "$port" "$key"
  if [ -n "$peer" ]; then
    printf '\n[Peer]\nPublicKey = %s\nAllowedIPs = %s\n' "$peer" "${allowed// /}"
  fi
}
if [ "$(cat "$conf" 2>/dev/null)" != "$(render)" ] || [ "$(stat -c '%a' "$conf" 2>/dev/null)" != "600" ]; then
  echo "$conf does not hold these settings yet."; exit 10
fi
if ! systemctl is-enabled --quiet wg-quick@wg0 || ! systemctl is-active --quiet wg-quick@wg0; then
  echo "wg-quick@wg0 is not enabled and running."; exit 10
fi
echo "WireGuard wg0 is up on UDP port $port."
