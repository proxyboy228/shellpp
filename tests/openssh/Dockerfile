# OpenSSH test server for the shellpp-ssh integration suite
# (SHELLPP_TEST_SSHD=openssh; docs/architecture.md, sections 9.1 and 9.3).
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

# - openssh-server brings ssh-keygen (openssh-client) and the SFTP server.
# - The package's postinst generates host keys; they are removed so that every
#   container generates its own.
# - pam_loginuid cannot write the login uid inside an unprivileged container;
#   as a required session module it would end every login after
#   authentication, so it becomes optional.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends openssh-server passwd; \
    rm -rf /var/lib/apt/lists/*; \
    rm -f /etc/ssh/ssh_host_*; \
    sed -i 's/^session[[:space:]]\+required[[:space:]]\+pam_loginuid\.so/session optional pam_loginuid.so/' /etc/pam.d/sshd; \
    if grep -q '^session[[:space:]]\+required[[:space:]]\+pam_loginuid' /etc/pam.d/sshd; then \
        echo 'pam_loginuid is still a required session module' >&2; exit 1; \
    fi; \
    mkdir -p /run/sshd

COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /usr/local/bin/shellpp-openssh-entrypoint
RUN chmod 0644 /etc/ssh/sshd_config \
    && chmod 0755 /usr/local/bin/shellpp-openssh-entrypoint

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/shellpp-openssh-entrypoint"]
