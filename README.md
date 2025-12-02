# Locket

> *A secrets management agent. Keeps your secrets safe, but out of sight.*

1. [Overview](#overview)
1. [Quick Start](#overview)
1. [Full Configuration](./docs/run.md)
1. [Permissions](#permissions)
1. [Examples](#example-hot-reloading-traefik-configurations-with-secrets)
1. [Supported Providers](#supported-providers)
1. [Roadmap](#roadmap)

## Overview
Locket is a small CLI tool, packaged as a tiny rootless and distroless Docker image, designed to be run as a sidecar process for applications which might carry sensitive data in configuration files. Locket can help keep sensitive files off disk completely in tmpfs, or just somewhere out of revision control.

The basic premise is:

1. Move your sensitive data to a dedicated secret manager (only 1password supported today, more to come), 
1. Adjust your config files to carry *secret references* instead of raw sensitive data, which are safe to commit directly to revision control (i.e `{{ op://vault/keys/privatekey?ssh-format=openssh }}`)
1. Configure locket to use your secrets provider `--provider=op` or with env: `SECRETS_PROVIDER=op`. Or just use the docker image tag `locket:op`
1. Mount your templates containing secret references for locket to read, i.e. `./templates:/templates:ro`, and mount an output directory for the secrets to be placed (usually a named tmpfs volume, or some secure location) `secrets-store:/run/secrets/`
1. Finally, map the template->output for each required mapping. You can map arbitrarily many directories->directories or files->files. `--map /templates:/run/secrets`

Your secrets will all be injected according to the provided configuration, and any dependant applications will have materialized secrets available.

> [!TIP] 
> By default, locket will also *watch* for changes to your secret reference files, and will reflect those changes immediately to the configured output. So if you have an application which supports a dynamic config file with hot-reloading, you can manage this with locket directly without downtime. If you dont want files watched, simply use `--mode=park` to inject once and then hang out (to keep the process alive for healthchecks). Or use `--mode=one-shot` to do a single inject and exit.

## Quick Start

We can use locket as a small sidecar service to safely inject secrets to tmpfs before our primary service starts.

A full configuration reference for all available options is provided in [`docs/run.md`](./docs/run.md)

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:latest
    user: "65532:65532" # The default user is 65532:65532 (nonroot) when not specified
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    # Configurations can be supplied via command like below, or via env variables.
    command:
        - "--provider=op"
        - "--op.token-file=/run/secrets/op_token"
        - "--map=/templates:/run/secrets" # Supports multiple maps, if needed.
        - "--secret=db_pass={{ op://vault/db/pass }}"
        - "--secret=db_host={{ op://vault/db/host }}"
        - "--secret=key={{ op://vault/keys/privatekey?ssh-format=openssh }}"
    secrets:
      - op_token
    volumes:
        # Mount in your actual secret templates, with secret references
      - ./config/templates:/templates:ro
        # Mount in your output directory, where you want secrets materialized
      - secrets-store:/run/secrets
  app:
    image: my-app:latest
    depends_on:
        locket:
            condition: healthy # locket is healthy once all secrets are injected
    volumes:
      # Mount the shared volume wherever you want the secrets in the container
      - secrets-store:/run/secrets:ro
    environment:
        # We can directly reference the materialized secrets as files
        DB_PASSWORD_FILE: /run/secrets/db_pass
        DB_HOST_FILE: /run/secrets/
        SECRET_KEY: /run/secrets/key

secrets:
  op_token:
    file: /etc/op/token # Must have read permissions by locket user

# We can create a shared tmpfs volume that locket will write to, and our app will
# read from
volumes:
  secrets-store:
    driver: local
    driver_opts:
      type: tmpfs
      device: tmpfs
```
## Security

The image runs as user `65532` (`nonroot`) by default. This was adopted from the standards
set in Google's popular rootless/distroless images. In addition, locket does not serve inbound requests and requires no elevated privilege. So it is safe to add any additional security measures to docker compose configuration.

It may be useful to explicitly set permissions on the tmpfs driver, to avoid any ambiguity. However, docker will typically set this up correctly when the volume is created, depending on what services depend on it.

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket
    user: "1000:1000"
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    volumes:
      - secrets-store:/run/secrets:ro

volumes:
  secrets-store:
    driver: local
    driver_opts:
      type: tmpfs
      device: tmpfs
      o: uid=1000,gid=1000,mode=700
```
## Example: Hot-Reloading Traefik configurations with Secrets

Traefik supports Dynamic Configuration via files, which it watches for changes. By pairing Traefik with Locket, you can inject secrets (like Dashboard credentials, TLS certificates, or middleware auth) into your configuration files and have Traefik hot-reload them automatically without a restart.

1. Locket watches a local `templates/` directory containing your Traefik config with `{{ op://... }}` placeholders.
1. When a template changes, Locket atomically updates the file in the shared secrets-store volume.
1. Traefik detects the change in the shared volume and reloads its configuration without a restart.

So a snippet from `./templates/dynamic_conf.yaml` might look like

```yaml
http:
  middlewares:
    auth:
      basicAuth:
        users:
          - "{{ op://DevOps/Traefik/basic_auth_user }}"

  routers:
    dashboard:
      rule: "Host(`traefik.localhost`)"
      service: "api@internal"
      middlewares: ["auth"]
# Any other secrets can be included here too....
```

```yaml
---
services:
  locket:
    image: ghcr.io/bpbradley/locket:op # Can use the 1pass specific tag
    container_name: locket
    user: "65532:65532" 
    environment:
      OP_SERVICE_ACCOUNT_TOKEN_FILE: /run/secrets/op_token
    secrets:
      - op_token
    command:
      - "--map=/templates:/run/secrets"
      - "--mode=watch"
    volumes:
      - ./templates:/templates:ro
      - secrets-store:/run/secrets

  traefik:
    image: traefik:v3
    container_name: traefik
    depends_on:
      locket:
        condition: service_healthy
    command:
      # Tell Traefik to watch the directory where Locket writes
      - "--providers.file.directory=/etc/traefik/dynamic"
      - "--providers.file.watch=true"
      - "--api.dashboard=true"
    ports:
      - 80:80
      - 443:443
      - 8080:8080
    volumes:
      # Mount the SHARED volume where Locket writes the 'real' config
      - secrets-store:/etc/traefik/dynamic:ro 
      - /var/run/docker.sock:/var/run/docker.sock:ro

secrets:
  op_token:
    file: /etc/op/token

volumes:
  # The bridge between Locket and Traefik.
  # Using tmpfs ensures secrets never touch the disk.
  secrets-store:
    driver: local
    driver_opts:
      type: tmpfs
      device: tmpfs
```

## Providers

1. 1password Connect (`--provider=op-connect`)
2. 1password Service Accounts (`--provider=op`)

> [!TIP]
> Each provider has its own docker image, if a slim version is preferred. The `latest` tag bundles all providers and their respective dependencies. But a provider specific tag like `locket:connect` is only about 4MB and has no extra dependencies besides what is needed for the connect provider.

> [!NOTE]
> The `op` (service account) provider is a bit more feature rich than the connect provider (currently), and is easier to setup, but it does require the bundled `op` cli dependency right now, because 1password does not offer a Rust SDK

## Roadmap

### Before v1.0.0

1. Have support for at least 4 providers
1. Add support for relative paths for more use as a standalone CLI.

### Beyond

1. **exec Command**: A wrapper mode (`locket exec --env .env -- docker compose up -d`) that injects secrets into the child process environment without writing files.
1. **Templating Engine**: Adding attributes to the secret reference which can transform secrets before injection. For example `{{ secret_reference | base64 }}` to encode the secret as base64, or `{{ secret_reference | totp }}` to interpret the secret as a totp code.
1. **Swarm Operator**: Native integration for Docker Swarm secrets.
