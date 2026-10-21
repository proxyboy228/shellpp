#!/bin/sh
# Variant no-sftp (tests/openssh/README.md, "Variants"): no SFTP subsystem.
set -eu
sed -i '/^Subsystem sftp/d' /etc/ssh/sshd_config
