# TopHat-managed: the agent layer of the dev container image.
ARG TOPHAT_BASE_IMAGE
FROM ${TOPHAT_BASE_IMAGE}

ARG CLAUDE_VERSION

# Claude Code at the pinned version, with its self-updater disabled. Its configuration home is left
# empty: the session mounts the project's own volume there.
ENV DISABLE_AUTOUPDATER=1
ENV CLAUDE_CONFIG_DIR=/opt/tophat/home/.claude
RUN set -eu; \
    test -n "$CLAUDE_VERSION"; \
    curl -fsSL https://claude.ai/install.sh | bash -s -- "$CLAUDE_VERSION"; \
    rm -rf "$HOME/.claude" "$HOME/.claude.json"; \
    claude --version; \
    chmod -R a+rX /opt/tophat; \
    find /opt/tophat/home -type d -exec chmod 1777 {} +
