#!/bin/sh
# TopHat-managed: the dev container launcher. Run it through ./th.sh from the project root.
#
#   ./th.sh            build the image (when needed) and start an interactive session
#   ./th.sh --check    run `tophat check` in a throwaway container
#   ./th.sh --dry-run  print the build and session commands without running them
#
# The container gets the workspace, the data roots and its own volumes, and nothing else from the
# host: no SSH keys or agent sockets, no git or cloud credentials, no git identity.

set -eu

usage() {
  cat <<'USAGE'
Usage: ./th.sh [--check | --dry-run]

  (no option)  Build the dev container image and start a session
  --check      Run `tophat check` in a throwaway container (the project is mounted read-only)
  --dry-run    Print the build and session commands without building or starting anything
USAGE
}

fail() {
  printf 'th.sh: %s\n' "$1" >&2
  exit 1
}

mode=session
case $# in
  0) ;;
  1)
    case "$1" in
      --check) mode=check ;;
      --dry-run) mode=dry ;;
      -h | --help) usage; exit 0 ;;
      *) usage >&2; exit 2 ;;
    esac
    ;;
  *) usage >&2; exit 2 ;;
esac

[ -d .tophat/managed ] || fail "run the launcher from the project root (./th.sh)"
command -v podman >/dev/null 2>&1 || fail "podman is not on PATH; install Podman 4.4 or later"

# A word printed in a command: quoted only when it needs it.
quote() {
  case "$1" in
    '' | *[!A-Za-z0-9_./:=,@%+-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}

# Runs a command, or prints it on one line in a dry run.
run() {
  if [ "$mode" = dry ]; then
    line=""
    for word in "$@"; do line="$line${line:+ }$(quote "$word")"; done
    printf '%s\n' "$line"
  else
    "$@"
  fi
}

# --- Project ID ---------------------------------------------------------------------------------

id_file=.tophat/project-id
if [ ! -e "$id_file" ]; then
  if [ -r /proc/sys/kernel/random/uuid ]; then
    new_id=$(cat /proc/sys/kernel/random/uuid)
  elif command -v uuidgen >/dev/null 2>&1; then
    new_id=$(uuidgen | tr 'A-F' 'a-f')
  else
    fail "cannot create $id_file: neither /proc/sys/kernel/random/uuid nor uuidgen is available"
  fi
  printf '%s\n' "$new_id" > "$id_file"
fi
id=$(head -n 1 "$id_file" | tr -d '\r')
printf '%s\n' "$id" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' ||
  fail "$id_file must hold a lowercase UUID, not '$id'; delete it to create a new one"
base_image="localhost/tophat-$id-base:latest"
agent_image="localhost/tophat-$id-agent:latest"
project_image="localhost/tophat-$id-project:latest"

# --- Run options --------------------------------------------------------------------------------

. .tophat/managed/launch/run-options.sh
run_options=$(emit_run_options .tophat/user/podman/run-options) || exit 1

# --- Mount sources ------------------------------------------------------------------------------

# Under WSL2 with the Windows podman binary, mount sources must be Windows paths.
host_path() {
  if grep -qi microsoft /proc/version 2>/dev/null; then
    case "$(command -v podman)" in
      *.exe | /mnt/*) wslpath -w "$1"; return ;;
    esac
  fi
  printf '%s\n' "$1"
}
workspace=$(host_path "$(pwd)")

# The host's TOPHAT_ROOT_<NAME> variables, passed by name.
root_vars=$(env | sed -n 's/^\(TOPHAT_ROOT_[A-Z0-9_]*\)=.*/\1/p' | sort)

# --- Build context ------------------------------------------------------------------------------

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) fail "unsupported architecture $(uname -m); TopHat images are built for x86_64 and aarch64" ;;
esac
target="$arch-unknown-linux-musl"

versions=.tophat/managed/container/versions.yaml
version_of() {
  value=$(sed -n "s/^$1: *//p" "$versions" | tr -d '"'"'"'\r')
  [ -n "$value" ] || fail "$versions has no $1"
  printf '%s\n' "$value"
}

pin=.tophat/managed/tophat-release.yaml
pinned=$(sed -n 's/^version: *//p' "$pin" | tr -d '"'"'"'\r')
context=.tophat/state/build
rm -rf "$context"
mkdir -p "$context/tophat-source"
if [ -n "${TOPHAT_DEV_BINARY:-}" ]; then
  [ -f "$TOPHAT_DEV_BINARY" ] || fail "TOPHAT_DEV_BINARY is '$TOPHAT_DEV_BINARY', which is not a file"
  cp "$TOPHAT_DEV_BINARY" "$context/tophat-source/tophat"
  dev_build=1
elif [ "$pinned" = unreleased ] || [ -z "$pinned" ]; then
  fail "no tophat release is pinned in $pin (version: unreleased). Set TOPHAT_DEV_BINARY to a Linux tophat binary, for example one built with 'cargo build --release' in the TopHat repository"
else
  url=$(sed -n 's/^url: *//p' "$pin" | tr -d '"'"'"'\r')
  sha=$(sed -n "s/^  $target: *//p" "$pin" | tr -d '"'"'"'\r')
  [ -n "$url" ] && [ -n "$sha" ] || fail "$pin has no url or no sha256 for $target"
  printf 'TOPHAT_URL=%s\nTOPHAT_SHA256=%s\n' "$url/tophat-$target" "$sha" > "$context/tophat-source/release.env"
  dev_build=0
fi
pixi_version=$(version_of pixi_version)
pixi_sha256=$(version_of "pixi_sha256_$arch")
copier_version=$(version_of copier_version)
claude_version=$(version_of claude_version)
cp .tophat/managed/container/Containerfile "$context/Containerfile"
cp .tophat/managed/container/Agent.Containerfile "$context/Agent.Containerfile"
cp .tophat/user/container/Containerfile "$context/Project.Containerfile"

build() {
  run podman build -t "$base_image" -f "$context/Containerfile" \
    --build-arg "PIXI_VERSION=$pixi_version" \
    --build-arg "PIXI_SHA256=$pixi_sha256" \
    --build-arg "PIXI_ARCH=$arch" \
    --build-arg "COPIER_VERSION=$copier_version" \
    --build-arg "TOPHAT_DEV_BUILD=$dev_build" \
    "$context" || fail "the base image could not be built; the previous image is unchanged"
  run podman build -t "$agent_image" -f "$context/Agent.Containerfile" \
    --build-arg "TOPHAT_BASE_IMAGE=$base_image" \
    --build-arg "CLAUDE_VERSION=$claude_version" \
    "$context" || fail "the agent image could not be built; the previous image is unchanged"
  run podman build -t "$project_image" -f "$context/Project.Containerfile" \
    --build-arg "TOPHAT_AGENT_IMAGE=$agent_image" \
    "$context" || fail "the project image could not be built (see .tophat/user/container/Containerfile)"
}

# --- Data roots ---------------------------------------------------------------------------------

# Asks tophat, in a throwaway container, where the data roots are, and sets root_mounts to one
# "<location>" per line for the configured ones. Read-only: the workspace is mounted ro.
read_roots() {
  set -- podman run --rm -v "$workspace:/work:ro" -w /work
  for var in $root_vars; do set -- "$@" -e "$var"; done
  roots=$("$@" "$project_image" tophat data roots 2>&1) ||
    fail "the data roots cannot be read: $roots"
  root_mounts=""
  tab=$(printf '\t')
  while IFS="$tab" read -r name location; do
    [ -n "$name" ] && [ -n "$location" ] && [ "$location" != - ] || continue
    [ -d "$location" ] ||
      fail "data root '$name' is located at $location, which does not exist on this host"
    root_mounts="$root_mounts$location
"
  done <<ROOTS
$roots
ROOTS
}

# --- Git hooks ----------------------------------------------------------------------------------

set_hooks() {
  git rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 0
  current=$(git config --local --get core.hooksPath || true)
  if [ -z "$current" ]; then
    git config --local core.hooksPath .tophat/managed/hooks
  elif [ "$current" != .tophat/managed/hooks ]; then
    printf 'th.sh: core.hooksPath is %s, so the TopHat pre-commit hook is not active; your hook can call .tophat/managed/hooks/pre-commit\n' "$current" >&2
  fi
}

# --- Commands -----------------------------------------------------------------------------------

# Prints its arguments, then the data-root mounts (each location with the given suffix) and the
# root variables, one word per line.
with_roots() {
  suffix=$1
  shift
  printf '%s\n' "$@"
  printf '%s' "$root_mounts" | while IFS= read -r location; do
    [ -n "$location" ] || continue
    printf '%s\n' -v "$(host_path "$location"):$location$suffix"
  done
  for var in $root_vars; do printf '%s\n' -e "$var"; done
}

case "$mode" in
  check)
    podman image exists "$project_image" || build
    read_roots
    command=$(with_roots ":ro" podman run --rm -v "$workspace:/work:ro" -w /work)
    set -f
    IFS='
'
    # shellcheck disable=SC2086
    set -- $command
    unset IFS
    set +f
    exec "$@" "$project_image" tophat check
    ;;
  dry)
    set_hooks
    build
    podman image exists "$project_image" ||
      fail "no image of this project exists yet; launch once first (./th.sh), then use --dry-run"
    read_roots
    ;;
  session)
    set_hooks
    build
    read_roots
    ;;
esac

command=$(with_roots "" podman run --rm -it -v "$workspace:/work" -w /work \
  -v "tophat-$id-pixi-cache:/opt/tophat/cache/pixi" \
  -v "tophat-$id-claude:/opt/tophat/home/.claude")
set -f
IFS='
'
# shellcheck disable=SC2086
set -- $command $run_options
unset IFS
set +f
run "$@" "$project_image" /work/.tophat/managed/launch/enter.sh
