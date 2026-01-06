# 1password Connect Provider

This provider is based on a [1password Connect](https://developer.1password.com/docs/connect/) backend. Requests for secrets are made via calls to the Connect REST API on your configured Connect host.

> [!NOTE]
> This provider requires a functioning and reachable 1password connect deployment. You cannot access `my.1password.com` using the Connect API

1. [Deploy a 1password Connect Server](https://developer.1password.com/docs/connect/get-started#step-2-deploy-a-1password-connect-server)
1. [Create an access token](https://developer.1password.com/docs/connect/manage-connect#create-a-token)
1. Configure locket with your reachable connect host URL, and your authentication token. [Configuration Reference](../inject.md#1password-connect)

## Example `locket inject` Configuration

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:connect
    user: "1000:1000"
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    container_name: locket-connect
    secrets:
      - connect_token
    volumes:
      - ./templates:/templates:ro
      - out-connect:/run/secrets/locket
    command: # Or use environment variables
      - "--connect.token=file:/run/secrets/connect_token"
      - "--connect.host=https://connect.example.com"
secrets:
  connect_token:
    file: /etc/tokens/connect
volumes:
  out-connect: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
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
        provider: op-connect
        connect.token: file:/etc/connect/token
        connect.host: $OP_CONNECT_HOST
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
