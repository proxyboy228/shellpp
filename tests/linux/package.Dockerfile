# The Linux package of the desktop app (D225, items 8 and 12;
# scripts/package-linux.sh): built on Ubuntu 22.04, so the executable needs
# glibc 2.35 or later (Ubuntu 22.04+, Debian 12+). One image for the release
# workflow (.github/workflows/release.yml) and the local dry run
# (scripts/release-dry-run.ps1), pinned by digest. The Rust toolchain is
# copied from the image tests/linux/Dockerfile pins (the version of
# rust-toolchain.toml; rustc itself needs only glibc 2.17); the system
# libraries are the ones GPUI links, as in CI (LINUX_GPUI_PACKAGES of
# .github/workflows/ci.yml) without libudev: the package is built without
# the `fido2-hid` feature (D173).
FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS rust

FROM ubuntu:22.04@sha256:0a9887931b0e38af2e3487ed39e29436b0535c4f3f52e57745baaf79d1898956

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates gcc g++ libc6-dev make perl pkg-config \
        libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libx11-xcb-dev \
        libfontconfig-dev libfreetype-dev libwayland-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rust /usr/local/rustup /usr/local/rustup
COPY --from=rust /usr/local/cargo /usr/local/cargo

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TARGET_DIR=/target \
    CARGO_TERM_COLOR=never
# rust-toolchain.toml names these components; without them rustup would
# fetch them at the first cargo call.
RUN rustup component add rustfmt clippy
WORKDIR /src
