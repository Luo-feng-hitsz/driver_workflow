#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Testing e1000 network..."
echo "nameserver 10.0.2.3" > /etc/resolv.conf
ifconfig eth0 up 2>/dev/null || true
ip link set eth0 up 2>/dev/null || true
sleep 1
wget --timeout=10 -q -O /dev/null http://bing.com && echo "wget SUCCESS" || echo "wget FAILED"
