# 1password Service Account Provider

This provider resolves secrets with the official 1Password SDK through `locket-op-bridge`, a small companion binary bundled with locket. 1Password does not publish a Rust SDK, so locket runs the SDK in a child process it fully owns. The bridge is spawned once, receives the service account token over a private pipe (never argv or env), resolves every batch of secrets in a single authenticated session, and exits with locket.

Compared to the previous `op` CLI backend this means:

1. **No `op` CLI dependency.** Nothing needs to be installed in images or on hosts, in any mode.
2. **Fast batch resolution.** One authenticated session serves all requests, instead of a full session handshake per secret.
3. **Runs as any user.** The bridge writes nothing to disk: no config directory, no `/tmp` state, no `/etc/passwd` requirements. `user: 1000:1000` (or any other UID) works with no workarounds.

> [!NOTE]
> `locket-op-bridge` is embedded in the prebuilt binaries and bundled in the Docker images, so nothing extra is needed. Only `cargo install locket` builds require downloading `locket-op-bridge-<arch>-<os>` from the releases page into the same directory as `locket`.

The [1password connect provider](./connect.md) remains available if you prefer a self-hosted Connect server.

## Setup

1. [Create a Service Account](https://developer.1password.com/docs/service-accounts/get-started#create-a-service-account)
1. Make sure to set permissions on the service account for the Vaults that it should have access to.
1. Store the Service Account token securely (i.e. in 1password)
1. Authenticate locket using the service account token via `--op-token` (or via env variables, supports raw token or `file:path/to/token`)

[Full configuration reference](../inject.md#1password-op)

```sh
locket inject --provider op \
  --op-token file:/path/to/token \
  --out /run/secrets/locket \
  --secret name={{op://Vault/Secret/Section/Item}} \
  --secret /path/to/secrets.yaml \
  --secret auth_key=@key.pem \
  --map ./tpl:/run/secrets/locket/mapped
```

# Example `locket inject` Configuration

Any `user:` works, including arbitrary non-root users:

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:op
    user: "1000:1000"
    container_name: locket
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    secrets:
      - op_token
    volumes:
      - ./templates:/templates:ro
      - out-op:/run/secrets/locket
    command: # Or use environment variables
      - "--op-token=file:/run/secrets/op_token"
secrets:
  op_token:
    file: /etc/tokens/op
volumes:
  out-op: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
```

## Example Provider Configuration

```yaml
---
name: provider
services:
  locket:
    provider:
      type: locket
      options:
        provider: op
        op-token: file:/etc/op/token
        secrets:
          - "secret1={{ op://Mordin/SecretPassword/Test Section/text }}"
          - "secret2={{ op://Mordin/SecretPassword/Test Section/date }}"
  demo:
    image: busybox
    user: "1000:1000"
    command: 
      - sh
      - -c
      - "env | grep LOCKET"
    depends_on:
      - locket
```

> [!NOTE]
> Provider mode uses the locket binary installed on the host. If it was installed with the installer script or a release tarball, the bridge is embedded and there is nothing extra to install. Only a `cargo install locket` build needs the separate `locket-op-bridge` download described above.
