# OpenBao / HashiCorp Vault Provider

This provider is based on [OpenBao](https://openbao.org/) (a Linux Foundation fork of HashiCorp Vault). It is also compatible with Vault Community Edition, since both share the same KV v2 and AppRole APIs.

It uses the KV v2 Secrets Engine to fetch secrets, and the AppRole auth method to authenticate.

> [!NOTE]
> This provider expects a running and unsealed OpenBao (or Vault) instance, which locket does not manage. See the [OpenBao documentation](https://openbao.org/docs/) for deployment and hardening guidance, and [Installing OpenBao](https://openbao.org/docs/install/) for installation options including container images.

## Reference syntax

`bao://<mount>/<path>/<field>`

* `mount`: the path where the KV v2 secrets engine is mounted (e.g. `secret`)
* `path`: the secret's path within that engine. May contain nested segments (e.g. `app/prod/db`)
* `field`: the specific key within the secret's data map

Example: `bao://secret/app/prod/db-password` refers to the `db-password` field of the secret stored at `app/prod` in the `secret` KV v2 mount.

> [!TIP]
> If multiple secret references point to the same `mount`/`path` (just different `field`s), locket will only fetch that secret once per resolution pass instead of once per field.


## Setup

1. Enable a KV v2 secrets engine (if not already enabled):

   ```sh
   bao secrets enable -path=secret -version=2 kv
   ```

2. Enable the AppRole auth method (if not already enabled):

   ```sh
   bao auth enable approle
   ```

3. Create a policy granting read access to the secrets locket needs. Note the `data/` segment between the mount and the secret path: that is how the KV v2 API addresses secret contents, so a policy written without it will not match.

   ```sh
   bao policy write locket - <<'EOF'
   path "secret/data/*" {
     capabilities = ["read"]
   }
   EOF
   ```

4. Create an AppRole role bound to that policy:

   ```sh
   bao write auth/approle/role/locket \
     token_policies="locket" \
     token_ttl=15m \
     token_max_ttl=30m \
     secret_id_ttl=0 \
     token_num_uses=0
   ```

   locket renews its token automatically when it expires, so short TTLs are fine. A role with `token_ttl=0` (non-expiring tokens) is also supported: locket logs in once and reuses the token.

5. Read the Role ID (not sensitive, safe to store alongside config):

   ```sh
   bao read auth/approle/role/locket/role-id
   ```

6. Generate a Secret ID and keep in a safe location

   ```sh
   bao write -f auth/approle/role/locket/secret-id
   ```

7. Provide the Secret ID to locket via `--bao-secret-id`. Prefer the `file:` form (e.g. a docker secret, as in the sidecar example below) over passing the value directly, so it stays out of process arguments and container environment.
[Here](../inject.md#openbao--vault-provider) is the reference configuration for locket using OpenBao/Vault

```sh
locket inject --provider bao \
  --bao-url https://openbao.example.com \
  --bao-role-id 00000000-0000-0000-0000-000000000000 \
  --bao-secret-id file:/path/to/secret-id \
  --out /run/secrets/locket \
  --secret "name={{bao://secret/app/prod/db-password}}" \
  --secret /path/to/secrets.yaml \
  --secret auth_key=@key.pem \
  --map ./tpl:/run/secrets/locket/mapped
```

## Example Sidecar Configuration

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:bao
    user: "1000:1000"
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    container_name: locket-bao
    secrets:
      - bao_secret_id
    volumes:
      - ./templates:/templates:ro
      - out-bao:/run/secrets/locket
    command: # Or use environment variables/TOML
      - "--bao-url=https://openbao.example.com"
      - "--bao-role-id=00000000-0000-0000-0000-000000000000"
      - "--bao-secret-id=file:/run/secrets/bao_secret_id"
secrets:
  bao_secret_id:
    file: /etc/tokens/bao-secret-id
volumes:
  out-bao: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
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
        provider: bao
        raw: true
        bao-url: "https://openbao.example.com"
        bao-role-id: "00000000-0000-0000-0000-000000000000"
        bao-secret-id: file:/etc/tokens/bao-secret-id
        env:
            - DB_PASSWORD={{bao://secret/app/prod/db-password}}
  demo:
    image: busybox
    user: "1000:1000"
    command:
      - sh
      - -c
      - "env | grep DB_PASSWORD"
    depends_on:
      - locket
```
