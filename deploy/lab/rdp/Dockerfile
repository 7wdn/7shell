FROM alpine:3.21

# xrdp plus a desktop to show once a session starts. Xvnc (tigervnc) is the backend because it
# needs no GPU or kernel driver inside a container; xfce is the smallest session that looks like a
# desktop rather than a test pattern.
RUN apk add --no-cache \
    dbus \
    openssl \
    tigervnc \
    ttf-dejavu \
    xfce4 \
    xfce4-terminal \
    xrdp

# Xvnc is the session backend: a container has no GPU and no display for Xorg, while Xvnc is a
# headless X server that needs neither. Removing the [Xorg] block leaves [Xvnc] as the first session
# section, which is what xrdp uses when a client auto-logs on without picking one.
RUN awk 'BEGIN { skip = 0 } /^\[Xorg\]/ { skip = 1; next } /^\[/ { skip = 0 } skip == 0 { print }' \
        /etc/xrdp/xrdp.ini > /tmp/xrdp.ini \
    && mv /tmp/xrdp.ini /etc/xrdp/xrdp.ini

# A certificate, or the server negotiates standard RDP security (RC4) and this client — which only
# accepts a TLS security layer — refuses the connection. That refusal is the point; the certificate
# is what makes the lab testable.
RUN openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout /etc/xrdp/key.pem -out /etc/xrdp/cert.pem -days 3650 -subj "/CN=7shell-lab" \
    && chmod 600 /etc/xrdp/key.pem \
    && sed -i 's|^certificate=$|certificate=/etc/xrdp/cert.pem|; s|^key_file=$|key_file=/etc/xrdp/key.pem|' /etc/xrdp/xrdp.ini

RUN adduser -D -s /bin/sh labuser && echo 'labuser:labpassword' | chpasswd

# The session xrdp starts for a login. `dbus-launch` is what makes an xfce session usable over the
# single display Xvnc provides.
RUN printf '%s\n' '#!/bin/sh' 'exec dbus-launch --exit-with-session startxfce4' > /etc/xrdp/startwm.sh \
    && chmod +x /etc/xrdp/startwm.sh

# Both daemons, because a container has no service manager. sesman starts the per-user Xvnc; xrdp
# is the listener that speaks RDP.
RUN printf '%s\n' '#!/bin/sh' 'set -e' 'xrdp-sesman --nodaemon &' 'exec xrdp --nodaemon' > /entrypoint.sh \
    && chmod +x /entrypoint.sh

EXPOSE 3389
ENTRYPOINT ["/entrypoint.sh"]
