# Dropbear (Alpine) for the compatibility matrix (docs/compatibility.md,
# tests/openssh/README.md "Compatibility images"). Test use only: its
# entrypoint creates the run's users (password and key; Dropbear has no
# keyboard-interactive) and fresh host keys.
#
#   docker build -f tests/openssh/compat/dropbear.Dockerfile -t shellpp-compat-dropbear:local tests/openssh
#   SHELLPP_OPENSSH_IMAGE=shellpp-compat-dropbear:local bash tests/openssh/start.sh <workdir>
FROM alpine:3.20

# openssh-sftp-server: Dropbear runs it for the sftp subsystem;
# openssh-keygen: the SHA256 fingerprints of the host keys; shadow: chpasswd.
RUN apk add --no-cache dropbear openssh-sftp-server openssh-keygen bash shadow coreutils

COPY compat/dropbear-entrypoint.sh /usr/local/bin/shellpp-dropbear-entrypoint
RUN chmod 0755 /usr/local/bin/shellpp-dropbear-entrypoint

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/shellpp-dropbear-entrypoint"]
