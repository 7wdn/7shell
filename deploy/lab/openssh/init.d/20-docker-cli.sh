#!/bin/sh
# The Docker CLI, for the SSH container live tests.
#
# The daemon is the `docker` service in compose.yaml; this container only needs the client. Installing
# at start-up rather than baking an image keeps the lab reproducible from the public OpenSSH image,
# and the check makes a restarted container a no-op.
#
# The wrapper is what makes a non-interactive SSH `docker ps` reach the sibling daemon: an SSH exec
# channel starts with almost no environment, so relying on DOCKER_HOST alone works for a login shell
# and fails for exactly the path this lab exists to test.
set -e

if ! command -v docker >/dev/null 2>&1; then
  apk add --no-cache docker-cli >/dev/null
fi

cat > /usr/local/bin/docker <<'WRAPPER'
#!/bin/sh
exec /usr/bin/docker -H "${DOCKER_HOST:-tcp://docker:2375}" "$@"
WRAPPER
chmod 0755 /usr/local/bin/docker
