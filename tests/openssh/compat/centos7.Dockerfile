# OpenSSH 7.4 (CentOS 7) for the compatibility matrix (docs/compatibility.md,
# tests/openssh/README.md "Compatibility images"). Test use only; the same
# entrypoint as the main image creates the run's users and host keys.
#
#   docker build -f tests/openssh/compat/centos7.Dockerfile -t shellpp-compat-centos7:local tests/openssh
#   SHELLPP_OPENSSH_IMAGE=shellpp-compat-centos7:local bash tests/openssh/start.sh <workdir>
#
# CentOS 7 reached its end of life: its packages come from vault.centos.org.
FROM centos:7

RUN set -eux; \
    sed -i -e 's/^mirrorlist=/#mirrorlist=/' \
        -e 's|^#baseurl=http://mirror.centos.org|baseurl=http://vault.centos.org|' \
        /etc/yum.repos.d/CentOS-*.repo; \
    yum install -y openssh-server passwd; \
    yum clean all; \
    rm -f /etc/ssh/ssh_host_*; \
    sed -i 's/^session\s\+required\s\+pam_loginuid.so/session optional pam_loginuid.so/' /etc/pam.d/sshd

COPY compat/centos7.sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /usr/local/bin/shellpp-openssh-entrypoint
COPY variants /etc/shellpp-variants
RUN chmod 0600 /etc/ssh/sshd_config \
    && chmod 0755 /usr/local/bin/shellpp-openssh-entrypoint /etc/shellpp-variants/*.sh

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/shellpp-openssh-entrypoint"]
