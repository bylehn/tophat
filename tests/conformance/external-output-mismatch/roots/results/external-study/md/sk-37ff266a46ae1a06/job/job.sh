#!/bin/sh
#SBATCH --cpus-per-task=1
#SBATCH --job-name=md
#
# External job for stage `md` of study `external-study`, key sk-37ff266a46ae1a06, written by tophat 0.1.0.
# You may edit the #SBATCH lines above. Any other change is detected when the folder returns.
# README.md has the steps.

set -eu
tophat_key='sk-37ff266a46ae1a06'
tophat_here() {
  [ -f "$1/inputs.sha256" ] && grep -F -q "$tophat_key" "$1/stage.json" 2>/dev/null
}
if [ -n "${SLURM_SUBMIT_DIR:-}" ] && tophat_here "$SLURM_SUBMIT_DIR"; then
  cd "$SLURM_SUBMIT_DIR"
elif tophat_here "$(dirname "$0")"; then
  cd "$(dirname "$0")"
else
  echo "job.sh: submit this script from its job folder: cd into the folder, then sbatch job.sh" >&2
  exit 2
fi

if command -v sha256sum >/dev/null 2>&1; then
  tophat_sha256() { tophat_h=$(sha256sum); printf '%s\n' "${tophat_h%% *}"; }
elif command -v shasum >/dev/null 2>&1; then
  tophat_sha256() { tophat_h=$(shasum -a 256); printf '%s\n' "${tophat_h%% *}"; }
else
  echo "job.sh: neither sha256sum nor shasum is available" >&2
  exit 2
fi
tophat_size() { printf '%s\n' "$(( $(wc -c < "$1") ))"; }
tophat_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
tophat_slashes() {
  tophat_n=0; tophat_r=$1
  while :; do
    case "$tophat_r" in
      */*) tophat_r=${tophat_r#*/}; tophat_n=$((tophat_n + 1)) ;;
      *) break ;;
    esac
  done
  printf '%s\n' "$tophat_n"
}
tophat_kept() {
  case "$1" in
    *.trr)
      if [ "$(tophat_slashes "$1")" -eq 0 ]; then return 0; fi ;;
  esac
  return 1
}

tophat_receipt() {
  {
    printf '%s\n' 'tophat-receipt 1' 'study external-study' 'stage md' "key $tophat_key"
    printf 'script %s\n' "$(grep -v '^#SBATCH' job.sh | tophat_sha256)"
    printf 'host %s\n' "$(uname -n)"
    printf 'job %s\n' "${SLURM_JOB_ID:--}"
    printf 'start %s\n' "$tophat_start"
    printf 'end %s\n' "$tophat_end"
    printf 'exit %s\n' "$1"
    printf '%s\n' "$tophat_modules" | while IFS= read -r tophat_m; do
      case "$tophat_m" in
        ''|*:|*' '*) ;;
        *) printf 'module %s\n' "$tophat_m" ;;
      esac
    done
    find out -type f | while IFS= read -r tophat_f; do
      tophat_rel=${tophat_f#out/}
      tophat_line="$(tophat_sha256 < "$tophat_f")  $(tophat_size "$tophat_f")  $tophat_rel"
      if tophat_kept "$tophat_rel"; then
        printf 'kept %s\n' "$tophat_line"
      else
        printf 'output %s\n' "$tophat_line"
      fi
    done
  } > .receipt.txt.tmp
  mv .receipt.txt.tmp receipt.txt
}

mkdir -p out
if [ -e receipt.txt ]; then
  echo "job.sh: receipt.txt exists, so this job already ran. To run it again, delete receipt.txt and the contents of out/" >&2
  exit 2
fi
if [ -n "$(ls -A out)" ]; then
  echo "job.sh: out/ is not empty. Delete its contents before submitting the job" >&2
  exit 2
fi

# Verify the inputs, parameters and code against the hashes tophat recorded.
tophat_bad=0
tophat_check() {
  if [ ! -f "$2" ] || [ "$(tophat_sha256 < "$2")" != "$1" ]; then
    printf 'job.sh: %s differs from the copy tophat wrote\n' "$2" >&2
    tophat_bad=1
  fi
}
tophat_check 'a2e6fc8214d69b536fddb7a626b3f7036a08c8a2576fa240532e587b14df2a27' 'inputs.sha256'
tophat_check 'ca3d163bab055381827226140568f3bef7eaac187cebd76878e0b63e9e442356' 'params.json'
tophat_check '5a7d6958673d6e4ef70a1fb9c25bcfa1db68150798009dc114ab2814d5d572a7' 'scripts/md.sh'
if [ -f inputs.sha256 ]; then
  while IFS= read -r tophat_l; do
    tophat_check "${tophat_l%%  *}" "${tophat_l#*  }"
  done < inputs.sha256
fi
if [ "$(( $(find inputs -type f 2>/dev/null | wc -l) ))" -ne 1 ]; then
  echo "job.sh: inputs/ holds files that tophat did not write" >&2
  tophat_bad=1
fi
tophat_modules=
if [ "$tophat_bad" -ne 0 ]; then
  tophat_start=-
  tophat_end=-
  tophat_receipt input-mismatch
  echo "job.sh: the job folder's inputs changed; the command was not run" >&2
  exit 1
fi

# Set up, run the command, and record what happened.
set +eu
tophat_status=0
if command -v module >/dev/null 2>&1; then
  tophat_modules=$(module -t list 2>&1)
fi
tophat_start=$(tophat_now)
if [ "$tophat_status" -eq 0 ]; then
  sh -c 'sh scripts/md.sh '\''out'\'''
  tophat_status=$?
fi
tophat_end=$(tophat_now)
set -eu
tophat_receipt "$tophat_status"
exit "$tophat_status"
