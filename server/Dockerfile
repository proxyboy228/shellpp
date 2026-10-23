# The Shell++ sync server image: a static musl binary on Alpine, run as an
# unprivileged user. Build from the repository root (the server depends on
# crates/sync-api):
#
#   docker build -f server/Dockerfile -t shellpp-server .
FROM rust:1.93-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# crates/sync-api inherits its package fields from the client workspace:
# keep that manifest with sync-api as its only member and without the
# client's patches.
COPY Cargo.toml Cargo.toml
RUN sed -i '/^members = \[/,/^\]/c\members = ["crates/sync-api"]' Cargo.toml \
    && sed -i '/^\[patch\.crates-io\]/,/^$/d' Cargo.toml
COPY crates/sync-api crates/sync-api
COPY server server
# The dev-dependencies (the client core, for the end-to-end tests) are not
# in the image; the lock file still pins every other version.
RUN awk '/^\[dev-dependencies\]/{skip=1;next} /^\[/{skip=0} !skip' server/Cargo.toml > /tmp/Cargo.toml \
    && mv /tmp/Cargo.toml server/Cargo.toml \
    && cargo build --manifest-path server/Cargo.toml --release \
    && strip server/target/release/shellpp-server

FROM alpine:3.22
RUN addgroup -S -g 10001 shellpp && adduser -S -D -H -u 10001 -G shellpp shellpp \
    && mkdir /data && chown shellpp:shellpp /data
COPY --from=build /src/server/target/release/shellpp-server /usr/local/bin/shellpp-server
USER shellpp
WORKDIR /data
ENV SHELLPP_BIND=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD wget -qO- http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["shellpp-server"]
CMD ["serve"]
