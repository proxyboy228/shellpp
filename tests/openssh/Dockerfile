# OpenSSH test server for the integration suites (shellpp-ssh with
# SHELLPP_TEST_SSHD=openssh, the desktop app's session tests) and the
# screenshot and benchmark runs of the desktop app (docs/architecture.md,
# sections 9.1 and 9.3).
#
# Test use only, never shipped. The image holds no users, passwords, keys or
# host keys: the entrypoint creates the users from the credentials that
# start.sh generates for each run and generates fresh host keys at container
# start (CONTRIBUTING.md, "Secrets and test data"). See README.md.
#
# The base image is pinned by the digest of its multi-platform index. To move
# to a newer Debian point release, see "Updating the base image" in README.md.
FROM debian:trixie-20261005-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

LABEL org.opencontainers.image.title="shellpp-test-openssh" \
      org.opencontainers.image.description="OpenSSH server with generated test credentials for the Shell++ integration tests"

# - openssh-server brings ssh-keygen and ssh-keyscan (openssh-client) and the
#   SFTP server.
# - htop and mc: full-screen programs for the terminal tests and screenshots
#   (alternate screen, mouse reporting, 256 colors, line drawing); procps
#   gives pkill for the control commands (container-control.sh).
# - tmux, vim, nano, less, emacs-nox and vttest: the terminal compatibility
#   checks (docs/compatibility.md, section 2).
# - busybox: a small HTTP server (`busybox httpd`) for the port forwarding
#   tests of the lab (lab.sh) and `nc`.
# - The package's postinst generates host keys; they are removed so that every
#   container generates its own.
# - pam_loginuid cannot write the login uid inside an unprivileged container;
#   as a required session module it would end every login after
#   authentication, so it becomes optional.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends openssh-server passwd htop mc procps busybox \
        tmux vim nano less emacs-nox vttest; \
    rm -rf /var/lib/apt/lists/*; \
    rm -f /etc/ssh/ssh_host_*; \
    sed -i 's/^session[[:space:]]\+required[[:space:]]\+pam_loginuid\.so/session optional pam_loginuid.so/' /etc/pam.d/sshd; \
    if grep -q '^session[[:space:]]\+required[[:space:]]\+pam_loginuid' /etc/pam.d/sshd; then \
        echo 'pam_loginuid is still a required session module' >&2; exit 1; \
    fi; \
    mkdir -p /run/sshd

# The test users' home directories (copied from /etc/skel by useradd):
# - UTF-8 sessions (C.UTF-8 is built into glibc; pam_env reads
#   /etc/default/locale for every SSH session), so htop and mc draw lines;
# - Debian's .bashrc already aliases `ls` to `ls --color=auto`; the prompt is
#   colored for every TERM, not only *-256color;
# - a few fictional files of different kinds, so that `ls --color` shows the
#   colors of directories, executables, symlinks, a broken symlink, archives
#   and images.
RUN set -eux; \
    printf 'LANG=C.UTF-8\n' >/etc/default/locale; \
    sed -i 's/^#force_color_prompt=yes/force_color_prompt=yes/' /etc/skel/.bashrc; \
    grep -q '^force_color_prompt=yes' /etc/skel/.bashrc; \
    grep -q "alias ls='ls --color=auto'" /etc/skel/.bashrc; \
    cd /etc/skel; \
    mkdir -p projects/website projects/api logs backups; \
    printf '# Sample project\n' >README.md; \
    printf '#!/bin/sh\necho "deploying"\n' >deploy.sh; \
    chmod 0755 deploy.sh; \
    printf 'started\nstopped\n' >logs/app.log; \
    : >backups/site-2026-10-01.tar.gz; \
    : >photo.png; \
    : >notes.txt; \
    ln -s projects/website current; \
    ln -s logs/missing.log broken-link

COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /usr/local/bin/shellpp-openssh-entrypoint
COPY container-control.sh /usr/local/bin/shellpp-openssh-control
# Variants of the server (README.md, "Variants"): scripts that edit the
# configuration at container start when SHELLPP_OPENSSH_VARIANT names one.
COPY variants /etc/shellpp-variants
RUN chmod 0644 /etc/ssh/sshd_config \
    && chmod 0755 /usr/local/bin/shellpp-openssh-entrypoint /usr/local/bin/shellpp-openssh-control \
    && chmod 0755 /etc/shellpp-variants/*.sh

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/shellpp-openssh-entrypoint"]
