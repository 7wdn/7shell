# A real registry for the login / push / pull live test

`registry:2` behind htpasswd, published on **127.0.0.1:5000** only. It exists so
`registry_login`, `image_push` and `image_pull` meet a server that asks for a password and stores
what it is given; it is a fixture, not something to publish to.

The credentials are deliberately trivial and are the ones the live test expects:
**`labuser` / `labpassword`**. `htpasswd` holds the bcrypt hash of that password (cost 5, so the
test's login is not slowed down by the fixture). Docker Desktop treats a registry on localhost as
insecure HTTP, so no certificate is needed; a registry anywhere else would need TLS instead.

## Running it

The live test starts its own container from `htpasswd`, so nothing has to be up first:

```bash
SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container --test live_registry -- --ignored --test-threads=1
```

If this compose recipe is already up, the test uses it instead (same port, same `htpasswd`, same
credentials) and leaves it running. To drive the fixture by hand:

```bash
docker compose -f deploy/lab/registry/compose.yaml up -d
curl -u labuser:labpassword http://127.0.0.1:5000/v2/          # 200 once authenticated
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:5000/v2/   # 401: auth is on
docker compose -f deploy/lab/registry/compose.yaml down
```

What the test does with it:

1. `registry_login` with `labpassword` succeeds; with any other password it fails with the CLI's
   `ContainerCommandFailed` and the refused password never appears in the error.
2. `busybox:1.36` is tagged `127.0.0.1:5000/sevenshell-live:<timestamp>`, pushed, its local tag is
   removed, and `image_pull` brings it back into `docker images`.

The test points `DOCKER_CONFIG` at a temporary directory for its own duration, so the login is
written there instead of into the developer's `~/.docker/config.json`, and the push depends on the
login the test performed rather than on something the machine already had.

Port 5000 is used because this machine's Windows reserves 1779-2978 and 9211-10010 for Hyper-V;
anything the lab publishes has to stay outside those ranges.

## Regenerating `htpasswd`

The hash can be produced by any bcrypt implementation; this one was made with:

```bash
python -c "import bcrypt; print('labuser:' + bcrypt.hashpw(b'labpassword', bcrypt.gensalt(5)).decode())"
```

Write the single resulting line into `htpasswd`; the registry reads it at start-up, so a running
container has to be restarted after a change.
