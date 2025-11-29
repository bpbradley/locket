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
1. Configure locket to use your secrets provider `--provider=op` or with env: `SECRETS_PROVIDER=op`
1. Mount your templates containing secret references for locket to read, i.e. `./templates:/template:ro`, and mount an output directory for the secrets to be placed (usually a named tmpfs volume, or some secure location) `secrets-store:/run/secrets/`
1. Finally, map the template->output for each required mapping. You can map arbitrarily many directories->directories or files->files. `--map /templates:/run/secrets`

Your secrets will all be injected according to the provided configuration, and any dependant applications will have materialized secrets available.

> [!TIP] By default, locket will also *watch* for changes to your secret reference files, and will reflect those changes immediately to the configured output. So if you have an application which supports a dynamic config file with hot-reloading, you can manage this with locket directly without downtime. If you dont want files watched, simply use `--mode=park` to inject once and then hang out (to keep the process alive for healthchecks). Or use `--mode=one-shot` to do a single inject and exit.

## Quick Start

We can use locket as a small sidecar service to safely inject secrets to tmpfs before our primary service starts.

A full configuration reference for all available options is provided in [`docs/run.md`](./docs/run.md)

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:latest
    user: "65532:65532" # The default user is 65532:65532 (nonroot) when not specified
    # Configurations can be supplied via command like below, or via env variables.
    command:
        - "--provider=op"
        - "--token-file=/run/secrets/op_token"
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
## Permissions

The image runs as user `65532` (`nonroot`) by default. This was adopted from the standards
set in Google's popular rootless/distroless images.

If you must run as a different user (e.g. uid: 1000), you will encounter strict security checks from the 1Password CLI (op). It requires that the current user has a valid entry in /etc/passwd and owns its configuration directory.

To support custom UIDs, you must mount two additional items

```yaml
services:
  locket:
    user: "1000:1000"
    volumes:
      # 1. Identity: Provide a user entry for UID 1000
      # i.e. `op:x:1000:1000::/home/nonroot:/bin/sh`
      - ./passwd:/etc/passwd:ro
      # 2. Config: Provide a writable config directory owned by UID 1000
      - ./op-data:/config
```

> [!NOTE] 
> This will be fixed in v1.0.0 when the `op` cli dependency is removed

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

1. 1password.

Yes, it is a lonely list. More providers will be supported prior to v1.0.0. The architecture was specifically designed to make sure this would be easy to expand later™

## Roadmap

### Before v1.0.0

1. Decouple from `op` cli dependancy in secrets provider. This was done as a convience while developing the broader architecture, but it carries some annoying caveats
1. Implement support for at least two more providers, as 1password is the only currently supported provider.
1. Add support for relative paths for more use as a standalone CLI.

### Beyond

1. Support for configuration via docker labels, so that one locket instance can exist and each
application / stack can easily add new secrets via docker labels
1. An `exec` subcommand which is able to wrap arbitrary commands, providing injected secrets as environment variables in the scope of the command. i.e. `locket exec --env .env -- docker compose up`, etc.
1. Support as a secrets operator for swarm mode

