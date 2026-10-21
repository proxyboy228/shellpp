# HTTP CONNECT and SOCKS5 proxies for the stage 5 lab (../lab.sh): tinyproxy
# with Basic authentication on 3128 and microsocks with username/password
# authentication on 1080 (Debian trixie has no Dante). Test use only, never shipped. The image holds no
# credentials: the entrypoint reads the ones lab.sh generates for each run.
FROM debian:trixie-20261005-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends tinyproxy microsocks; \
    rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/shellpp-proxy-entrypoint
RUN chmod 0755 /usr/local/bin/shellpp-proxy-entrypoint
EXPOSE 3128 1080
ENTRYPOINT ["/usr/local/bin/shellpp-proxy-entrypoint"]
