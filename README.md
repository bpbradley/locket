# locket

> *A secrets management agent. Keeps your secrets safe, but out of sight.*

[![Build Status](https://github.com/bpbradley/locket/actions/workflows/ci.yml/badge.svg)](https://github.com/bpbradley/locket/actions)
[![Crates.io](https://img.shields.io/crates/v/locket.svg)](https://crates.io/crates/locket)
[![Docker](https://img.shields.io/github/v/release/bpbradley/locket?sort=semver&label=docker&logo=docker)](https://github.com/bpbradley/locket/pkgs/container/locket)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](LICENSE)

1. [Overview](#overview)
1. [Supported Providers](#providers)
1. [Full Configuration](./docs/CONFIGURATION.md)
1. [Roadmap](#roadmap)

## Overview
locket is a small CLI tool, packaged as a tiny rootless and distroless Docker image, designed to orchestrate secrets for dependent applications and services. locket is designed to work with most secrets providers, and it will orchestrate the retrieval of secrets and injection of them into dependent services. locket can help keep sensitive files off disk completely in tmpfs, or just somewhere out of revision control.

Currently, locket operates in two modes for two distinct purposes.

1. [Sidecar mode](#sidecar-mode): Inject secrets into configuration files stored in a shared, ephemeral tmpfs volume. locket will render files with secret references replaced with actual secrets so that dependent services can use them.
1. [Provider mode](#provider-mode): locket can be installed as a Docker CLI plugin, and it will inject secrets directly into the dependent process enviornment before it starts.

## Providers

1. [1password Connect](./docs/providers/connect.md)
2. [1password Service Accounts](./docs/providers/op.md)
3. [Bitwarden Secrets Manager](./docs/providers/bws.md)

> [!TIP]
> Each provider has its own docker image for sidecar mode, if a slim version is preferred. The `latest` tag bundles all providers and their respective dependencies. But a provider specific tag like `locket:connect` is only about 4MB and has no extra dependencies besides what is needed for the connect provider.

## Sidecar Mode

The basic premise of locket as a sidecar service is:

1. Move your sensitive data to a dedicated secret manager ([Supported Providers](#providers))
1. Adjust your config files to carry *secret references* instead of raw sensitive data, which are safe to commit directly to revision control (i.e `{{ op://vault/keys/privatekey?ssh-format=openssh }}`)
1. Configure locket to use your secrets provider `--provider=bws` or with env: `SECRETS_PROVIDER=bws`. Or just use the docker image tag `locket:bws`
1. Mount your templates containing secret references for locket to read, i.e. `./templates:/templates:ro`, and mount an output directory for the secrets to be placed (usually a named tmpfs volume, or some secure location) `secrets-store:/run/secrets/locket`
1. Finally, map the template->output for each required mapping. You can map arbitrarily many directories->directories or files->files. `--map /templates:/run/secrets/locket`

Your secrets will all be injected according to the provided configuration, and any dependant applications will have materialized secrets available.

> [!TIP] 
> By default, locket will also *watch* for changes to your secret reference files, and will reflect those changes immediately to the configured output. So if you have an application which supports a dynamic config file with hot-reloading, you can manage this with locket directly without downtime. If you dont want files watched, simply use `--mode=park` to inject once and then hang out (to keep the process alive for healthchecks). Or use `--mode=one-shot` to do a single inject and exit.

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
        - "--provider=op-connect"
        - "--op.token-file=/run/secrets/op_token"
        - "--map=/templates:/run/secrets/locket" # Supports multiple maps, if needed.
        - "--secret=db_pass={{ op://vault/db/pass }}"
        - "--secret=db_host={{ op://vault/db/host }}"
        - "--secret=key={{ op://vault/keys/privatekey?ssh-format=openssh }}"
    secrets:
      - op_token
    volumes:
        # Mount in your actual secret templates, with secret references
      - ./config/templates:/templates:ro
        # Mount in your output directory, where you want secrets materialized
      - secrets-store:/run/secrets/locket
  app:
    image: my-app:latest
    depends_on:
        locket:
            condition: healthy # locket is healthy once all secrets are injected
    volumes:
      # Mount the shared volume wherever you want the secrets in the container
      - secrets-store:/run/secrets/locket:ro
    environment:
        # We can directly reference the materialized secrets as files
        DB_PASSWORD_FILE: /run/secrets/locket/db_pass
        DB_HOST_FILE: /run/secrets/locket/db_host
        SECRET_KEY: /run/secrets/locket/key

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

### Security

The sidecar image runs as user `65532` (`nonroot`) by default. This was adopted from the standards set in Google's popular rootless/distroless images. In addition, locket does not serve inbound requests and requires no elevated privilege. So it is safe to add any additional security measures to docker compose configuration.

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
      - secrets-store:/run/secrets/locket:ro

volumes:
  secrets-store:
    driver: local
    driver_opts:
      type: tmpfs
      device: tmpfs
      o: uid=1000,gid=1000,mode=700
```

## Provider mode

 locket can be installed as a docker CLI plugin, and be used as a [Docker Compose provider service](https://docs.docker.com/compose/how-tos/provider-services/). In this mode, locket manages the `compose up` lifecycle. Every time `docker compose up` is called, `locket compose up` is first called by Docker, where locket will take provided secret references and set them as environment variables in the dependent container.

 A full configuration reference for all available options is provided in [`docs/compose.md`](./docs/compose.md)

```yaml
---
name: provider
services:
  locket:
    provider:
      type: locket
      options:
        provider: op-connect
        connect.token-file: /etc/connect/token
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

> [!NOTE]
> The environment variables are injected with the providers service name prefixed.
> This is behavior managed by Docker directly, and cannot be changed. So in some cases it may be necessary to expand the environment variable in the container like `$$APPLICATION_SECRET`.

In order to use the Provider mode, `locket` must be installed on the host system directly as a Docker CLI plugin. The simplest way to do this is to install the binary directly from GitHub, and symlink it to the appropriate directory for docker to access it as a cli-plugin.

### Install prebuilt binaries

The install script will install `locket` to your user home directory, as well as a `locket-update` script.

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bpbradley/locket/releases/latest/download/locket-installer.sh | sh
```

Otherwise, install the prebuilt binary directly for your architecture. The script above will install for the correct architecture automatically.

|  File  | Platform | Checksum |
|--------|----------|----------|
| [locket-aarch64-apple-darwin.tar.xz](https://github.com/bpbradley/locket/releases/latest/download/locket-aarch64-apple-darwin.tar.xz) | Apple Silicon macOS | [checksum](https://github.com/bpbradley/locket/releases/latest/download/locket-aarch64-apple-darwin.tar.xz.sha256) |
| [locket-x86_64-apple-darwin.tar.xz](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-apple-darwin.tar.xz) | Intel macOS | [checksum](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-apple-darwin.tar.xz.sha256) |
| [locket-aarch64-unknown-linux-gnu.tar.xz](https://github.com/bpbradley/locket/releases/latest/download/locket-aarch64-unknown-linux-gnu.tar.xz) | ARM64 Linux | [checksum](https://github.com/bpbradley/locket/releases/latest/download/locket-aarch64-unknown-linux-gnu.tar.xz.sha256) |
| [locket-x86_64-unknown-linux-gnu.tar.xz](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-unknown-linux-gnu.tar.xz) | x64 Linux | [checksum](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-unknown-linux-gnu.tar.xz.sha256) |
| [locket-x86_64-unknown-linux-musl.tar.xz](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-unknown-linux-musl.tar.xz) | x64 MUSL Linux | [checksum](https://github.com/bpbradley/locket/releases/latest/download/locket-x86_64-unknown-linux-musl.tar.xz.sha256) |

### Symlink locket binary to docker-locket as a Docker CLI Plugin

1. Confirm `locket` is installed with `locket --version`
1. Make sure a cli-plugins directory exists `mkdir -p ~/.docker/cli-plugins`
1. Symlink locket -> cli-plugins/locket `ln -sf $(which locket) ~/.docker/cli-plugins/docker-locket`
1. Confirm docker sees it. `docker info | grep locket`

## Example: Hot-Reloading Traefik configurations with Secrets

Traefik supports Dynamic Configuration via files, which it watches for changes. By pairing Traefik with locket, you can inject secrets (like Dashboard credentials, TLS certificates, or middleware auth) into your configuration files and have Traefik hot-reload them automatically without a restart.

1. locket watches a local `templates/` directory containing your Traefik config with `{{ op://... }}` placeholders.
1. When a template changes, locket atomically updates the file in the shared secrets-store volume.
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
      - "--map=/templates:/run/secrets/locket"
      - "--mode=watch"
    volumes:
      - ./templates:/templates:ro
      - secrets-store:/run/secrets/locket

  traefik:
    image: traefik:v3
    container_name: traefik
    depends_on:
      locket:
        condition: service_healthy
    command:
      # Tell Traefik to watch the directory where locket writes
      - "--providers.file.directory=/etc/traefik/dynamic"
      - "--providers.file.watch=true"
      - "--api.dashboard=true"
    ports:
      - 80:80
      - 443:443
      - 8080:8080
    volumes:
      # Mount the SHARED volume where locket writes the 'real' config
      - secrets-store:/etc/traefik/dynamic:ro 
      - /var/run/docker.sock:/var/run/docker.sock:ro

secrets:
  op_token:
    file: /etc/op/token

volumes:
  # The bridge between locket and Traefik.
  # Using tmpfs ensures secrets never touch the disk.
  secrets-store:
    driver: local
    driver_opts:
      type: tmpfs
      device: tmpfs
```

## Roadmap

### Before v1.0.0

1. Have support for at least 4 providers
1. **exec Command**: A wrapper mode (`locket exec --env .env -- docker compose up -d`) that injects secrets into the child process environment without writing files.
1. **Templating Engine**: Adding attributes to the secret reference which can transform secrets before injection. For example `{{ secret_reference | base64 }}` to encode the secret as base64, or `{{ secret_reference | totp }}` to interpret the secret as a totp code.

### Beyond

1. **Swarm Operator**: Native integration for Docker Swarm secrets.
