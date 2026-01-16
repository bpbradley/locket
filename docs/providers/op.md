# 1password Service Account Provider

This provider is based on the 1password `op` CLI as a backend. Each request for a secret is handled via a subprocess call to `op read`. This is unfortunately the only reasonable means of supporting 1password service accounts, because 1password does not provider a Rust SDK. It works generally just fine, but carries some side effects.

1. Because each request for a secret is executed via a subprocess call to `op read`, for injection of many secret files, this can take some time (about 0.5 seconds to process each file, compared to milliseconds for other providers). For each file, secrets are pre-collected and then batch requested in batches of 10 at a time, so it takes the same amount of time to resolve 10 secrets as it does 1. But secrets are batched per-file, not collected in advance and then batch processed. This means that it can take a material amount of time to inject secrets if many config files are needed.
1. The `op` CLI has very strict permissions checks in place on its configuration directory which cannot be overridden. Most of these are handled by the default configuration, but if a non-root user *besides* the default user (`nonroot(65532):nonroot(65532)`), it will not work without a [workaround](#workaround-for-arbitrary-non-root-users).
1. If using locket as a standalone CLI and need the `op` provider, you will also need to have the `op` binary installed on system.

If these issues are not satisfactory, the [1password connect provider](./connect.md) does not carry these issues

> [!NOTE]
> Because of the above constraints, using `op` provider in Provider mode (where `locket`) is installed directly on the host, means that you must also have the `op` CLI installed.

## Setup

1. [Create a Service Account](https://developer.1password.com/docs/service-accounts/get-started#create-a-service-account)
1. Make sure to set permissions on the service account for the Vaults that it should have access to.
1. Store the Service Account token securely (i.e. in 1password)
1. Authenticate locket using the service account token via `--op-token` (or via env variables, supports raw token or `file:path/to/token`)
1. If using provider mode, [install `op` cli](https://developer.1password.com/docs/cli/get-started/) dependency. 


[Full configuration reference](../inject.md#1password-op)

```sh
locket inject --provider op \
  --op-token file:/path/to/token \\
  --out /run/secrets/locket \
  --secret name={{op://Vault/Secret/Section/Item}}
  --secret /path/to/secrets.yaml \
  --secret auth_key=@key.pem \
  --map ./tpl:/run/secrets/locket/mapped 
```

# Example `locket inject` Configuration

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

## Workaround for arbitrary non-root users

To run as an arbitrary non-root user (i.e. `1000:1000`), the `$XDG_CONFIG_DIR/op` directory must:

1. Have strict `0700` permissions
1. Be owned by the running user (i.e. `1000:1000`)
1. Be owned by a *named user* resolvable by `/etc/passwd`

Because every user cannot be exhaustively added to `/etc/passwd`, one must be supplied which is able to satisfy `op`. The only users available by default are the default `nonroot` and `root`.

Here is an example configuration which will work with `user: 1000:1000`

```yaml
# Create cfg volume with defined permissions. Can also bind mount one if permissions are correct
volumes:
  op-cfg: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
  out-op: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }

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
      - op-cfg:/config:rw # Mount config directory
      - /etc/passwd:/etc/passwd:ro # Mount /etc/passwd (can be a separate, stripped down version with just one user if desired)
    command:
      - "--op-token=file:/run/secrets/op_token"
```

## Example Provider Configuration


> [!IMPORTANT]
> If using `op` in provider mode, you must have `op` cli installed on your system as well.

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
