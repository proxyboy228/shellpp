# The Linux build and tests of the client in Docker (run 12;
# scripts/verify-linux.ps1): the system libraries GPUI links on Linux, as the
# CI workflow installs them (LINUX_GPUI_PACKAGES of .github/workflows/ci.yml),
# on the Rust image of the toolchain rust-toolchain.toml pins. The source is
# mounted read-only and the target folder and cargo's registry are volumes.
FROM rust:1.99.1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev \
        libx11-xcb-dev libfontconfig-dev libfreetype-dev libwayland-dev \
        libudev-dev \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add rustfmt clippy

ENV CARGO_TARGET_DIR=/target \
    CARGO_TERM_COLOR=never \
    SHELLPP_TEST=1 \
    SHELLPP_TEST_SSHD=russh
WORKDIR /src
