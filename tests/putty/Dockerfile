# PuTTY's puttygen for the key import tests of shellpp-core (stage 3,
# crates/core/tests/key_formats_putty.rs): .ppk files of version 2 and 3,
# plain and encrypted, of keys that puttygen generates in the container at
# test time (CONTRIBUTING.md, "Secrets and test data"). Test use only,
# never shipped. The base image is the one of tests/openssh (pinned by
# digest); see tests/openssh/README.md, "Updating the base image".
FROM debian:trixie-20261005-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

LABEL org.opencontainers.image.title="shellpp-test-putty" \
      org.opencontainers.image.description="puttygen for the Shell++ key import tests"

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends putty-tools; \
    rm -rf /var/lib/apt/lists/*

COPY generate.sh /usr/local/bin/generate.sh
RUN chmod 0755 /usr/local/bin/generate.sh

ENTRYPOINT ["/usr/local/bin/generate.sh"]
