#!/bin/sh
# TopHat-managed: starts a session inside the dev container. With arguments, it runs them instead
# of the interactive shell.

if ! pixi install --locked; then
  echo "TopHat: the environment could not be installed: pixi.toml and pixi.lock disagree, or a package could not be fetched (see above)." >&2
fi
if [ "${TOPHAT_DEV_BUILD:-0}" = 1 ]; then
  echo "TopHat: this session runs a development build of $(tophat --version)."
fi
tophat status || true

if [ "$#" -gt 0 ]; then
  exec "$@"
fi
exec bash
