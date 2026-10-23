#!/bin/sh
# TopHat-managed: starts the project's dev container. The launcher itself lives in
# .tophat/managed/launch/th.sh.
cd "$(dirname "$0")" || exit 2
exec sh .tophat/managed/launch/th.sh "$@"
