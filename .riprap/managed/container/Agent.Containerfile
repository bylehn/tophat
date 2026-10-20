ARG RIPRAP_TOOLING_IMAGE
FROM ${RIPRAP_TOOLING_IMAGE}


ARG CLAUDE_VERSION




# The installers place files beneath HOME, which the tooling image sets to an
# image-owned path outside /root. Each agent's own configuration home under that
# path is a credential mount point at run time, so the image leaves nothing there.

RUN set -eu; \
    test -n "$CLAUDE_VERSION"; \
    curl -fsSL https://claude.ai/install.sh | bash -s -- "$CLAUDE_VERSION"; \
    rm -rf "$HOME/.claude.json" "$HOME/.claude"







ENV DISABLE_AUTOUPDATER=1

# Reach the agent programs as any unprivileged user, on the same terms as the toolchain the tooling
# image installed. Every directory beneath the home directory is made sticky world-writable so any
# runtime user can create the scratch and configuration directories the agents write at startup --
# an agent installed as root leaves directories there that the runtime user must write into.
RUN chmod -R a+rX /opt/riprap && find /opt/riprap/home -type d -exec chmod 1777 {} +
