# The Linux package in a clean system (D225, item 3; scripts/verify-linux-desktop.ps1):
# Ubuntu 22.04, the oldest system the package supports, with only the
# run-time libraries README lists (no -dev packages, so a missing one shows),
# plus what stands in for a desktop: Xvfb, Mesa's OpenGL through EGL
# (llvmpipe, no GPU), a D-Bus session and GNOME Keyring as the Secret
# Service, and the tools of the check (xdotool, ImageMagick's import,
# secret-tool, desktop-file-validate).
#
# Not Mesa's software Vulkan driver (lavapipe): GPUI's renderer (wgpu)
# picks it over OpenGL, and under Xvfb its frames never reach the screen,
# which stays black (run 17); with no Vulkan driver wgpu uses OpenGL.
FROM ubuntu:22.04@sha256:0a9887931b0e38af2e3487ed39e29436b0535c4f3f52e57745baaf79d1898956

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        libxkbcommon0 libxkbcommon-x11-0 libxcb1 libx11-xcb1 libwayland-client0 \
        libfontconfig1 libfreetype6 fonts-dejavu-core \
        libegl1 libegl-mesa0 libgl1-mesa-dri \
        xvfb xdotool x11-utils imagemagick \
        dbus dbus-x11 gnome-keyring libsecret-tools \
        desktop-file-utils ca-certificates procps openssh-server \
    && rm -rf /var/lib/apt/lists/*

# The check starts as root (OpenSSH as the test server, with a password made
# at run time) and runs the app as `tester`.
RUN useradd --create-home --shell /bin/bash tester \
    && useradd --create-home --shell /bin/bash sshuser \
    && mkdir -p /run/sshd
WORKDIR /tmp
