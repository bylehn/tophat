#!/bin/sh
# Applies a mean and a median filter of odd width $3 to every signal in directory $1, writing
# $2/<signal>.tsv with the columns clean, mean and median (stage `filter`). It needs only a POSIX
# shell and awk, so it runs on a cluster from the stage's job folder.
set -eu

[ "$#" -eq 3 ] || { echo "usage: filter.sh <signals dir> <out dir> <width>" >&2; exit 2; }
in=$1
out=$2
width=$3
mkdir -p "$out"
found=0
for f in "$in"/*.tsv; do
  [ -f "$f" ] || continue
  found=1
  awk -v w="$width" '
    BEGIN { FS = "\t"; OFS = "\t"; h = int(w / 2) }
    NR == 1 { next }
    { n++; clean[n] = $1; noisy[n] = $2 }
    END {
      print "clean", "mean", "median"
      for (i = 1; i <= n; i++) {
        lo = i - h; if (lo < 1) lo = 1
        hi = i + h; if (hi > n) hi = n
        k = 0; sum = 0
        for (j = lo; j <= hi; j++) { k++; win[k] = noisy[j] + 0; sum += noisy[j] }
        for (a = 2; a <= k; a++) {
          v = win[a]; b = a - 1
          while (b >= 1 && win[b] > v) { win[b + 1] = win[b]; b-- }
          win[b + 1] = v
        }
        if (k % 2) med = win[(k + 1) / 2]; else med = (win[k / 2] + win[k / 2 + 1]) / 2
        printf "%s\t%.6f\t%.6f\n", clean[i], sum / k, med
      }
    }' "$f" > "$out/${f##*/}"
done
[ "$found" -eq 1 ] || { echo "filter.sh: no .tsv signals in $in" >&2; exit 1; }
