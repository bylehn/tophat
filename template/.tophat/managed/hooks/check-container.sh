#!/bin/sh
# TopHat-managed: exits 0 inside a container; outside one, prints a block decision for the agent
# and exits 2, so an agent started on the host refuses to work. The first argument is the system
# root to inspect (default /), so tests can simulate hosts.

root=${1:-/}
root=${root%/}

in_container() {
  [ -f "$root/.dockerenv" ] && return 0
  [ -f "$root/run/.containerenv" ] && return 0
  [ "${container:-}" = podman ] && return 0
  [ -n "${APPTAINER_CONTAINER:-}" ] || [ -n "${SINGULARITY_CONTAINER:-}" ] && return 0
  [ -d "$root/.singularity.d" ] && return 0
  grep -qE 'docker|libpod|lxc|kubepods' "$root/proc/1/cgroup" 2>/dev/null && return 0
  awk '$5 == "/" && $9 == "overlay" { found = 1; exit } END { exit !found }' \
    "$root/proc/self/mountinfo" 2>/dev/null && return 0
  [ -n "${KUBERNETES_SERVICE_HOST:-}" ] && return 0
  return 1
}

if in_container; then
  exit 0
fi
cat <<'JSON'
{
  "decision": "block",
  "reason": "TopHat: the agent must run inside the project's dev container. Start a session with ./th.sh and run the agent there."
}
JSON
exit 2
