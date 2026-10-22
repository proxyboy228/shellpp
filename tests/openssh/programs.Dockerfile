# The terminal programs of the compatibility screenshots (docs/compatibility.md,
# section 2; D83, item 7) on top of the main test image: tmux, vim, nano,
# less, emacs-nox and vttest. Only the screenshot run's program states
# (scripts/ui-screens.ps1) use it; the session tests and the server matrix
# keep the smaller main image.
#
# Built by start.sh when SHELLPP_OPENSSH_PROGRAMS=1, from the image start.sh
# runs otherwise (BASE): the same users, entrypoint and variants.
#
# Test use only, never shipped; nothing secret is part of it (README.md).
ARG BASE=shellpp-test-openssh:local
FROM ${BASE}

LABEL org.opencontainers.image.title="shellpp-test-openssh-programs" \
      org.opencontainers.image.description="The Shell++ OpenSSH test server with terminal programs for the compatibility screenshots"

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends tmux vim nano less emacs-nox vttest; \
    rm -rf /var/lib/apt/lists/*
