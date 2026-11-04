#!/bin/sh
# TopHat-managed: detects the host's GPU, printing nvidia, nvidia-no-cdi, amd or none.
#
# The optional first argument is a system root (default /), so tests can simulate hosts.

root=${1:-/}

if [ -e "$root/dev/nvidiactl" ]; then
  for dir in "$root/etc/cdi" "$root/var/run/cdi"; do
    [ -d "$dir" ] || continue
    for spec in "$dir"/*; do
      [ -f "$spec" ] || continue
      if grep -q 'nvidia\.com/gpu' "$spec" 2>/dev/null; then
        printf 'nvidia\n'
        exit 0
      fi
    done
  done
  printf 'nvidia-no-cdi\n'
  exit 0
fi

if [ -e "$root/dev/kfd" ] && [ -d "$root/dev/dri" ]; then
  for node in "$root/dev/dri"/renderD*; do
    if [ -e "$node" ] || [ -L "$node" ]; then
      printf 'amd\n'
      exit 0
    fi
  done
fi

printf 'none\n'
