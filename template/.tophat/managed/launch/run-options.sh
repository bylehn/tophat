#!/bin/sh
# TopHat-managed: validates a run-options file and prints each accepted option on its own line.
#
# One option per line, so an option reaches the runtime exactly as written. Blank lines and `#`
# comments are ignored. A line holding whitespace, or not starting with "-", is refused with the
# file and line number. A missing file yields nothing.
emit_run_options() {
  _rofile=$1
  [ -f "$_rofile" ] || return 0
  _roline=0
  while IFS= read -r _raw || [ -n "$_raw" ]; do
    _roline=$((_roline + 1))
    _opt=$(printf '%s' "$_raw" | sed 's/\r$//; s/^[[:space:]]*//; s/[[:space:]]*$//')
    case "$_opt" in
      '' | '#'*) continue ;;
    esac
    case "$_opt" in
      *[[:space:]]*)
        printf 'th.sh: %s line %s: an option must be one word without spaces: %s\n' \
          "$_rofile" "$_roline" "$_opt" >&2
        return 1 ;;
    esac
    case "$_opt" in
      -*) ;;
      *)
        printf "th.sh: %s line %s: an option must start with '-': %s\n" \
          "$_rofile" "$_roline" "$_opt" >&2
        return 1 ;;
    esac
    printf '%s\n' "$_opt"
  done < "$_rofile"
}
