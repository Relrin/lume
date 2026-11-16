# An OpenSSH server to test SSH panes against. Not for anything else: the account's
# password comes from the environment and the host keys are made anew at every start.
FROM alpine:3.22

RUN apk add --no-cache openssh-server bash \
    && adduser -D -s /bin/sh lume \
    && mkdir -p /home/lume/.ssh \
    && chown lume:lume /home/lume/.ssh \
    && chmod 700 /home/lume/.ssh

COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh

EXPOSE 22
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
