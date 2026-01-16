# BWS Provider

This provider is based on the [Bitwarden Secrets Manager](https://bitwarden.com/products/secrets-manager/). It uses the offical `bitwarden` rust crate as the backend.

> [!NOTE]
> This is **not** the same as a Bitwarden/Vaultwarden provider. A Bitwarden Vault Management API based provider is significantly more complex because it is not designed for machine access.

1. [Setup a Bitwarden Secrets Manager account](https://bitwarden.com/help/secrets-manager-quick-start/)
1. [Add a machine account with access to desired secrets / projects](https://bitwarden.com/help/secrets-manager-quick-start/#add-a-machine-account)
1. Configure locket with your machine token. [Configuration Reference](../inject.md#bitwarden-secrets-provider)

```sh
locket inject --provider bws --bws-token=file:/path/to/token
    --out /run/secrets/locket
    --secret=/path/to/secrets.yaml
    --secret=auth_key=@key.pem
    --map ./tpl:/run/secrets/locket/mapped
```

## Example `locket inject` Configuration

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:bws
    user: "1000:1000"
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    container_name: locket-bws
    secrets:
      - bws_token
    volumes:
      - ./templates:/templates:ro
      - out-bws:/run/secrets/locket
    command: # Or use environment variables
      - "--bws-token=file:/run/secrets/bws_token"
secrets:
  bws_token:
    file: /etc/tokens/bws
volumes:
  out-bws: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
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
        provider: bws
        bws-token: file:/etc/bws/token
        secrets:
          - "secret1={{ 3832b656-a93b-45ad-bdfa-b267016802c3 }}"
          - "secret2={{ 3e0f2247-b359-4408-83d0-b3a70152731c }}"
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
