#!/bin/sh
# TopHat-managed: the dev container launcher. Run it through ./th.sh from the project root.
#
#   ./th.sh             build the image (when needed) and start an interactive session
#   ./th.sh --check     run `tophat check` in a throwaway container
#   ./th.sh --dry-run   print the commands without building or starting anything
#   ./th.sh --export-sif  build the image and export it as an Apptainer SIF file
#   ./th.sh --run-sif     start a session from the exported SIF file under Apptainer
#
# The container gets the workspace, the data roots, its own volumes and the human's personal skills
# and instructions, and nothing else from the host: no SSH keys or agent sockets, no git or cloud
# credentials, no git identity.

set -eu

usage() {
  cat <<'USAGE'
Usage: ./th.sh [--check | --dry-run] [--export-sif | --run-sif]

  (no option)   Build the dev container image and start a Podman session
  --check       Run `tophat check` in a throwaway container (the project is mounted read-only)
  --dry-run     Print the commands without building or starting anything
  --export-sif  Build the image and export it as an Apptainer SIF file
  --run-sif     Start a session from the exported SIF file under Apptainer
USAGE
}

fail() {
  printf 'th.sh: %s\n' "$1" >&2
  exit 2
}

mode=session
dry=0
for argument in "$@"; do
  case "$argument" in
    --check | --export-sif | --run-sif)
      if [ "$mode" = session ]; then
        mode=${argument#--}
      else
        usage >&2
        exit 2
      fi
      ;;
    --dry-run) dry=1 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
[ "$mode" = check ] && [ "$dry" = 1 ] && { usage >&2; exit 2; }

[ -d .tophat/managed ] || fail "run the launcher from the project root (./th.sh)"
if [ "$mode" != run-sif ]; then
  command -v podman >/dev/null 2>&1 || fail "podman is not on PATH; install Podman 4.4 or later"
fi

# A word printed in a command: quoted only when it needs it.
quote() {
  case "$1" in
    '' | *[!A-Za-z0-9_./:=,@%+-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}

# Runs a command, or prints it on one line in a dry run.
run() {
  if [ "$dry" = 1 ]; then
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
sif=.tophat/state/apptainer/tophat-$id.sif

# --- Run options --------------------------------------------------------------------------------

. .tophat/managed/launch/run-options.sh
if [ "$mode" = run-sif ]; then
  run_options_file=.tophat/user/apptainer/run-options
else
  run_options_file=.tophat/user/podman/run-options
fi
run_options=$(emit_run_options "$run_options_file") || exit 2

# --- GPU ----------------------------------------------------------------------------------------

gpu_setting=
gpu_options=

# rq-380bb3d9
# Reads the GPU setting: TOPHAT_GPU when set, otherwise .tophat/user/container/gpu, otherwise
# auto. Any other value is refused.
resolve_gpu_setting() {
  if [ "${TOPHAT_GPU+set}" = set ]; then
    gpu_setting=$TOPHAT_GPU
  elif [ -f .tophat/user/container/gpu ]; then
    gpu_setting=$(sed -n '1p' .tophat/user/container/gpu | tr -d '\r' |
      sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
  else
    gpu_setting=auto
  fi
  case "$gpu_setting" in
    auto | on | off) ;;
    *) fail "the GPU setting '$gpu_setting' is invalid; use auto, on or off" ;;
  esac
}

# rq-e363eed4
# Prints the detected GPU, or none on macOS, where no GPU can be passed through.
detect_gpu() {
  if [ "$(uname -s 2>/dev/null || true)" = Darwin ]; then
    printf 'none\n'
  else
    sh .tophat/managed/launch/gpu-detect.sh
  fi
}

# rq-4cf7a956 rq-5cf97dd8
# Sets gpu_options for the runtime (podman or apptainer) from the setting and the detected GPU,
# refusing or explaining when no usable GPU can be passed through.
prepare_gpu() {
  runtime=$1
  resolve_gpu_setting
  gpu_options=
  [ "$gpu_setting" = off ] && return 0
  detected=$(detect_gpu) || fail "the GPU could not be detected"
  case "$detected" in
    nvidia)
      if [ "$runtime" = apptainer ]; then
        gpu_options=--nv
      else
        gpu_options="--device nvidia.com/gpu=all"
      fi
      ;;
    amd)
      if [ "$runtime" = apptainer ]; then
        gpu_options=--rocm
      else
        gpu_options="--device /dev/kfd --device /dev/dri --group-add keep-groups"
      fi
      ;;
    nvidia-no-cdi)
      # Apptainer's --nv needs no CDI specification.
      if [ "$runtime" = apptainer ]; then
        gpu_options=--nv
        return 0
      fi
      message="the NVIDIA GPU needs a CDI specification; create one with: sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml"
      if [ "$gpu_setting" = on ]; then
        fail "$message"
      fi
      printf 'th.sh: %s; the session runs on CPU\n' "$message" >&2
      ;;
    none)
      if [ "$gpu_setting" = on ]; then
        fail "the GPU setting is 'on', but no usable GPU was found; use auto or off to run on CPU"
      fi
      printf 'th.sh: no usable GPU was found; the session runs on CPU\n' >&2
      ;;
    *)
      fail "gpu-detect.sh printed '$detected', which is not a known GPU kind"
      ;;
  esac
}

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

if [ "$mode" != run-sif ]; then
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
codex_version=$(version_of codex_version)
codex_sha256=$(version_of "codex_sha256_$arch")
opencode_version=$(version_of opencode_version)
opencode_sha256=$(version_of "opencode_sha256_$arch")
case "$arch" in
  x86_64) opencode_arch=x64 ;;
  aarch64) opencode_arch=arm64 ;;
esac
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
    --build-arg "TOPHAT_PROJECT_ID=$id" \
    "$context" || fail "the base image could not be built; the previous image is unchanged"
  run podman build -t "$agent_image" -f "$context/Agent.Containerfile" \
    --build-arg "TOPHAT_BASE_IMAGE=$base_image" \
    --build-arg "CLAUDE_VERSION=$claude_version" \
    --build-arg "CODEX_VERSION=$codex_version" \
    --build-arg "CODEX_SHA256=$codex_sha256" \
    --build-arg "CODEX_ARCH=$arch" \
    --build-arg "OPENCODE_VERSION=$opencode_version" \
    --build-arg "OPENCODE_SHA256=$opencode_sha256" \
    --build-arg "OPENCODE_ARCH=$opencode_arch" \
    "$context" || fail "the agent image could not be built; the previous image is unchanged"
  run podman build -t "$project_image" -f "$context/Project.Containerfile" \
    --build-arg "TOPHAT_AGENT_IMAGE=$agent_image" \
    "$context" || fail "the project image could not be built (see .tophat/user/container/Containerfile)"
}
fi

# --- Data roots ---------------------------------------------------------------------------------

# Turns the output of `tophat data roots` into root_mounts, one "<location>" per line for the
# configured roots outside the workspace.
collect_roots() {
  roots=$1
  root_mounts=""
  tab=$(printf '\t')
  while IFS="$tab" read -r name location; do
    [ -n "$name" ] && [ -n "$location" ] && [ "$location" != - ] || continue
    # A location inside the project (such as the default results/) is in the workspace mount.
    case "$location" in /work | /work/*) continue ;; esac
    [ -d "$location" ] ||
      fail "data root '$name' is located at $location, which does not exist on this host"
    root_mounts="$root_mounts$location
"
  done <<ROOTS
$roots
ROOTS
}

# Asks tophat, in a throwaway container, where the data roots are. Read-only: the workspace is
# mounted ro.
read_roots() {
  set -- podman run --rm -v "$workspace:/work:ro" -w /work
  for var in $root_vars; do set -- "$@" -e "$var"; done
  roots=$("$@" "$project_image" tophat data roots 2>&1) ||
    fail "the data roots cannot be read: $roots"
  collect_roots "$roots"
}

# Reads the data roots from the exported SIF image, without building anything.
read_roots_sif() {
  set -- apptainer exec --containall --no-home --cleanenv --writable-tmpfs \
    --bind "$workspace:/work:ro" --pwd /work
  for var in $root_vars; do
    eval "value=\${$var-}"
    set -- "$@" --env "$var=$value"
  done
  roots=$("$@" "$sif" tophat data roots 2>&1) ||
    fail "the data roots cannot be read from $sif: $roots"
  collect_roots "$roots"
}

# --- Local tools ------------------------------------------------------------------------------

# Asks tophat, in a throwaway container, which tools are declared, and sets tool_mounts to one host
# path per line for the local tool repositories (absolute paths and file:// URLs), which the session
# mounts read-only so `tophat tools sync` can fetch from them. Problems only warn: the session is
# where the human fixes them.
read_tools() {
  tool_mounts=""
  set -- podman run --rm -v "$workspace:/work:ro" -w /work
  if ! tools=$("$@" "$project_image" tophat tools list 2>&1); then
    printf 'th.sh: warning: the tools cannot be read, so no local tool is mounted: %s\n' "$tools" >&2
    return 0
  fi
  while IFS= read -r line; do
    case "$line" in '' | ' '*) continue ;; esac
    name=${line%%  *}
    url=${line#*  }
    case "$url" in
      file://*) path=${url#file://} ;;
      /*) path=$url ;;
      *) continue ;;
    esac
    case "$path" in /work | /work/*) continue ;; esac
    if [ -d "$path" ]; then
      tool_mounts="$tool_mounts$path
"
    else
      printf "th.sh: warning: tool '%s' is declared at %s, which does not exist on this host; it is not mounted\n" "$name" "$path" >&2
    fi
  done <<TOOLS
$tools
TOOLS
}

# --- Selected agents -----------------------------------------------------------------------------

# The agents the `agents` Copier question selected, one per line. Copier records the answers in
# the project's .copier-answers.yml; without that file no agent is selected.
selected_agents() {
  answers=.copier-answers.yml
  if [ ! -f "$answers" ]; then
    printf 'th.sh: warning: %s does not exist, so no agent is selected\n' "$answers" >&2
    return 0
  fi
  # Copier writes the list either inline ([a, b]) or as one YAML line per entry (- a).
  inline=$(sed -n 's/^agents: *//p' "$answers" | head -n 1)
  case "$inline" in
    \[*\])
      printf '%s\n' "$inline" | tr -d '[]"' | tr ',' '\n' | sed 's/^ *//;s/ *$//'
      ;;
    '')
      sed -n '/^agents:$/,/^[^ -]/p' "$answers" | sed -n 's/^ *- *//p' | tr -d '"'
      ;;
  esac
}

# Whether the `agents` Copier question selected the agent named by the argument.
agent_selected() {
  selected_agents | grep -qx "$1"
}

# rq-1cc9f489
# Prints one "-v name:home" per line: a named volume at each selected agent's configuration home,
# holding that agent's own login and session state across sessions. Unknown agents only warn.
agent_volume_mounts() {
  selected_agents | while IFS= read -r agent; do
    [ -n "$agent" ] || continue
    case "$agent" in
      claude-code) name=claude; home=/opt/tophat/home/.claude ;;
      codex) name=codex; home=/opt/tophat/home/.codex ;;
      opencode) name=opencode; home=/opt/tophat/home/.local/share/opencode ;;
      *)
        printf 'th.sh: warning: agent %s is unknown; no volume is mounted for it\n' "$agent" >&2
        continue
        ;;
    esac
    printf '%s\n' -v "tophat-$id-$name:$home"
  done
}

# rq-4f95e12e
# Prints one "--bind host:home" per line for each selected agent's Apptainer configuration home,
# a directory under .tophat/state/apptainer/agents that belongs to the invoking user alone.
agent_state_mounts() {
  selected_agents | while IFS= read -r agent; do
    [ -n "$agent" ] || continue
    case "$agent" in
      claude-code) home=/opt/tophat/home/.claude ;;
      codex) home=/opt/tophat/home/.codex ;;
      opencode) home=/opt/tophat/home/.local/share/opencode ;;
      *)
        printf 'th.sh: warning: agent %s is unknown; no configuration home is mounted for it\n' "$agent" >&2
        continue
        ;;
    esac
    directory=.tophat/state/apptainer/agents/$agent
    mkdir -p "$directory"
    chmod 700 "$directory"
    printf '%s\n' --bind "$(pwd)/$directory:$home"
  done
}

# --- Personal skills and instructions -----------------------------------------------------------

# Prints the human's TopHat configuration folder, or nothing when neither XDG_CONFIG_HOME nor HOME
# is set.
personal_config() {
  if [ -n "${XDG_CONFIG_HOME:-}" ]; then
    printf '%s\n' "$XDG_CONFIG_HOME/tophat"
  elif [ -n "${HOME:-}" ]; then
    printf '%s\n' "$HOME/.config/tophat"
  fi
}

# Prints the container directories where the selected agents read personal skills, one per line
# and without duplicates. Claude Code reads ~/.claude/skills; Codex and OpenCode both read
# ~/.agents/skills (rqm/template/container.md, Personal skills).
personal_skill_destinations() {
  if agent_selected claude-code; then
    printf '%s\n' /opt/tophat/home/.claude/skills
  fi
  if agent_selected codex || agent_selected opencode; then
    printf '%s\n' /opt/tophat/home/.agents/skills
  fi
}

# Prints each selected agent's user-level instructions file in the container, one per line.
personal_instruction_destinations() {
  if agent_selected claude-code; then
    printf '%s\n' /opt/tophat/home/.claude/CLAUDE.md
  fi
  if agent_selected codex; then
    printf '%s\n' /opt/tophat/home/.codex/AGENTS.md
  fi
  if agent_selected opencode; then
    printf '%s\n' /opt/tophat/home/.config/opencode/AGENTS.md
  fi
}

# rq-c490844e
# Sets skill_mounts to one "<host path>:<container destination>" per line for the skills in the
# human's personal skills folder, which the session mounts read-only in each selected agent's
# user-level skills directory, one by one. Entries that cannot be mounted only warn. Without the
# folder it does nothing.
read_skills() {
  skill_mounts=""
  destinations=$(personal_skill_destinations)
  [ -n "$destinations" ] || return 0
  config=$(personal_config)
  [ -n "$config" ] || return 0
  folder=$config/skills
  [ -d "$folder" ] || return 0
  for entry in "$folder"/*; do
    [ -e "$entry" ] || continue
    name=${entry##*/}
    skip=""
    if [ ! -d "$entry" ]; then
      skip="it is not a folder"
    elif [ ! -f "$entry/SKILL.md" ]; then
      skip="it has no SKILL.md"
    else
      case "$name" in
        *[!a-z0-9-]*) skip="its name is not lowercase letters, digits and hyphens" ;;
      esac
    fi
    if [ -z "$skip" ]; then
      for project_skills in .claude/skills .agents/skills .opencode/skills; do
        if [ -e "$project_skills/$name" ]; then
          skip="it has the name of a project skill"
          break
        fi
      done
    fi
    if [ -z "$skip" ]; then
      # A symbolic link is followed: its target is what gets mounted.
      path=$(cd -P "$entry" && pwd -P)
      case "$path" in *[:,]* | *'
'*) skip="its path $path contains ':' or ','" ;; esac
    fi
    if [ -n "$skip" ]; then
      printf "th.sh: warning: personal skill '%s' in %s is not mounted: %s\n" "$name" "$folder" "$skip" >&2
      continue
    fi
    for destination in $destinations; do
      skill_mounts="$skill_mounts$path:$destination/$name
"
    done
  done
}

# rq-59768c6a
# Sets instruction_mounts to one "<host path>:<container destination>" per line for the human's
# personal CLAUDE.md, which the session mounts read-only as each selected agent's user-level
# instructions, or to nothing when there is none or it cannot be mounted (which only warns).
read_instructions() {
  instruction_mounts=""
  destinations=$(personal_instruction_destinations)
  [ -n "$destinations" ] || return 0
  config=$(personal_config)
  [ -n "$config" ] || return 0
  file=$config/CLAUDE.md
  [ -e "$file" ] || [ -L "$file" ] || return 0
  # A symbolic link is followed: its target is what gets mounted.
  target=$file
  hops=0
  while [ -L "$target" ] && [ "$hops" -lt 40 ]; do
    hops=$((hops + 1))
    link=$(readlink "$target")
    case "$link" in
      /*) target=$link ;;
      *) target=$(dirname "$target")/$link ;;
    esac
  done
  skip=""
  if [ -L "$target" ]; then
    skip="its symbolic links form a loop"
  elif [ ! -f "$target" ]; then
    skip="it is not a file"
  else
    target=$(cd -P "$(dirname "$target")" && pwd -P)/$(basename "$target")
    case "$target" in *[:,]* | *'
'*) skip="its path $target contains ':' or ','" ;; esac
  fi
  if [ -n "$skip" ]; then
    printf 'th.sh: warning: personal instructions %s are not mounted: %s\n' "$file" "$skip" >&2
    return 0
  fi
  for destination in $destinations; do
    instruction_mounts="$instruction_mounts$target:$destination
"
  done
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
  export-sif)
    command -v apptainer >/dev/null 2>&1 ||
      fail "apptainer is not on PATH; install Apptainer 1.2 or later"
    set_hooks
    build
    mkdir -p .tophat/state/apptainer
    archive=.tophat/state/apptainer/tophat-$id.oci.tar
    temporary=$sif.tmp.$$
    if [ "$dry" = 1 ]; then
      run podman save --format oci-archive -o "$archive" "$project_image"
      run apptainer build "$temporary" "oci-archive:$archive"
      exit 0
    fi
    rm -f "$archive" "$temporary"
    if ! podman save --format oci-archive -o "$archive" "$project_image"; then
      rm -f "$archive" "$temporary"
      fail "the image could not be saved as an OCI archive; $sif is unchanged"
    fi
    if ! apptainer build "$temporary" "oci-archive:$archive"; then
      rm -f "$archive" "$temporary"
      fail "the SIF image could not be built; $sif is unchanged"
    fi
    rm -f "$archive"
    mv "$temporary" "$sif"
    exit 0
    ;;
  run-sif)
    [ -f "$sif" ] ||
      fail "no exported image at $sif; create one with './th.sh --export-sif' on a build host and copy it to the same path"
    command -v apptainer >/dev/null 2>&1 ||
      fail "apptainer is not on PATH; install Apptainer 1.2 or later"
    labels=$(apptainer inspect --labels "$sif" 2>&1) ||
      fail "the image labels cannot be read from $sif: $labels"
    project_label=$(printf '%s\n' "$labels" |
      sed -n 's/^org\.tophat\.project-id:[[:space:]]*//p' | head -n 1 | tr -d '\r')
    if [ -z "$project_label" ]; then
      project_label=$(printf '%s\n' "$labels" |
        sed -n 's/^org\.tophat\.project-id=[[:space:]]*//p' | head -n 1 | tr -d '\r')
    fi
    if [ "$project_label" != "$id" ]; then
      fail "the image $sif is labelled for project '${project_label:-none}', not '$id'; export it from this project with './th.sh --export-sif'"
    fi
    set_hooks
    read_roots_sif
    read_skills
    read_instructions
    agent_state_binds=$(agent_state_mounts)
    prepare_gpu apptainer
    ;;
  session)
    set_hooks
    build
    if [ "$dry" = 1 ]; then
      podman image exists "$project_image" ||
        fail "no image of this project exists yet; launch once first (./th.sh), then use --dry-run"
    fi
    read_roots
    read_tools
    read_skills
    read_instructions
    prepare_gpu podman
    ;;
esac

case "$mode" in
  session)
    command=$(
      with_roots "" podman run --rm -it -v "$workspace:/work" -w /work \
        -v "tophat-$id-pixi-cache:/opt/tophat/cache/pixi"
      agent_volume_mounts
      printf '%s' "$tool_mounts" | while IFS= read -r path; do
        [ -n "$path" ] && printf '%s\n' -v "$(host_path "$path"):$path:ro"
      done
      # rq-c490844e
      printf '%s' "$skill_mounts" | while IFS= read -r mount; do
        [ -n "$mount" ] || continue
        printf '%s\n' -v "$(host_path "${mount%:*}"):${mount##*:}:ro"
      done
      # rq-59768c6a
      printf '%s' "$instruction_mounts" | while IFS= read -r mount; do
        [ -n "$mount" ] || continue
        printf '%s\n' -v "$(host_path "${mount%:*}"):${mount##*:}:ro"
      done
    )
    set -f
    IFS='
'
    # shellcheck disable=SC2086
    set -- $command $gpu_options $run_options
    unset IFS
    set +f
    run "$@" "$project_image" /work/.tophat/managed/launch/enter.sh
    ;;
  run-sif)
    command=$(
      printf '%s\n' apptainer exec --containall --no-home --cleanenv --writable-tmpfs \
        --bind "$workspace:/work" --pwd /work
      printf '%s' "$root_mounts" | while IFS= read -r location; do
        [ -n "$location" ] || continue
        printf '%s\n' --bind "$(host_path "$location"):$location"
      done
      for var in $root_vars; do
        eval "value=\${$var-}"
        printf '%s\n' --env "$var=$value"
      done
      printf '%s\n' "$agent_state_binds"
      # rq-c490844e
      printf '%s' "$skill_mounts" | while IFS= read -r mount; do
        [ -n "$mount" ] || continue
        printf '%s\n' --bind "$(host_path "${mount%:*}"):${mount##*:}:ro"
      done
      # rq-59768c6a
      printf '%s' "$instruction_mounts" | while IFS= read -r mount; do
        [ -n "$mount" ] || continue
        printf '%s\n' --bind "$(host_path "${mount%:*}"):${mount##*:}:ro"
      done
    )
    set -f
    IFS='
'
    # shellcheck disable=SC2086
    set -- $command $gpu_options $run_options
    unset IFS
    set +f
    run "$@" "$sif" env HOME=/opt/tophat/home /work/.tophat/managed/launch/enter.sh
    ;;
esac
