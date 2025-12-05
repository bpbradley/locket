# BWS Provider

This provider is based on the [Bitwarden Secrets Manager](https://bitwarden.com/products/secrets-manager/). It uses the offical `bitwarden` rust crate as the backend.

> [!NOTE]
> This is **not** the same as a Bitwarden/Vaultwarden provider. A Bitwarden Vault Management API based provider is significantly more complex because it is not designed for machine access.

1. [Setup a Bitwarden Secrets Manager account](https://bitwarden.com/help/secrets-manager-quick-start/)
1. [Add a machine account with access to desired secrets / projects](https://bitwarden.com/help/secrets-manager-quick-start/#add-a-machine-account)
1. Configure locket with your machine token. [Configuration Reference](../run.md#bitwarden-secrets-provider)

# Example Docker Compose

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
      - "--bws.token-file=/run/secrets/bws_token"
secrets:
  bws_token:
    file: /etc/tokens/bws
volumes:
  out-bws: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
```
