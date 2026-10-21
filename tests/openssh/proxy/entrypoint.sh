#!/bin/sh
# Starts tinyproxy (HTTP CONNECT, Basic authentication) and microsocks (SOCKS5,
# username/password) with the credentials of /run/shellpp-input/proxy.user
# and proxy.password. Both reach only what the container's networks reach.
set -eu

input=/run/shellpp-input
lifetime="${SHELLPP_OPENSSH_LIFETIME:-3600}"
[ -s "$input/proxy.user" ] && [ -s "$input/proxy.password" ] || {
    echo 'shellpp-proxy: missing credentials' >&2
    exit 1
}
user=$(head -n 1 "$input/proxy.user")
password=$(head -n 1 "$input/proxy.password")
socks_password="$password"

cat >/etc/tinyproxy/tinyproxy.conf <<CONF
Port 3128
Listen 0.0.0.0
Timeout 60
MaxClients 50
Allow 0.0.0.0/0
BasicAuth $user $password
LogLevel Warning
CONF
password=''

rm -rf "$input"
tinyproxy -c /etc/tinyproxy/tinyproxy.conf
exec timeout "$lifetime" microsocks -i 0.0.0.0 -p 1080 -u "$user" -P "$socks_password"
