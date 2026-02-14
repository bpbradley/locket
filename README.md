# locket

> *A secrets management agent. Keeps your secrets safe and out of sight.*

[![Build Status](https://github.com/bpbradley/locket/actions/workflows/ci.yml/badge.svg)](https://github.com/bpbradley/locket/actions)
[![Crates.io](https://img.shields.io/crates/v/locket.svg)](https://crates.io/crates/locket)
[![Docker](https://img.shields.io/github/v/release/bpbradley/locket?sort=semver&label=docker&logo=docker)](https://github.com/bpbradley/locket/pkgs/container/locket)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](LICENSE)

1. [Overview](#overview)
1. [Supported Providers](#providers)
1. [Full Configuration](./docs/CONFIGURATION.md)
1. [Roadmap](#roadmap)

## Overview
locket is a small CLI tool (also packaged as a tiny rootless and distroless Docker image) designed to orchestrate secrets for dependent applications and services. locket is designed to work with most secrets providers, and it will coordinate the retrieval of secrets and injection of them into dependent services.

locket is a versatile tool and it supports various forms of secrets injection.

1. [Secrets Injection](./docs/inject.md): Materialize secrets from templates into files using `locket inject`
1. [Container Sidecar](#sidecar-mode): Inject secrets into configuration files stored in a shared, ephemeral tmpfs volume. locket will render files with secret references replaced with actual secrets so that dependent services can use them.
1. [Provider](#provider-mode): locket can be installed as a Docker CLI plugin, and it will inject secrets directly into the dependent process enviornment before it starts.
1. [Orchestrator](#orchestration): `locket exec` is able to manage a specified subcommand, injecting secrets into its process environment. It can also watch for changes to environment files, and restart the dependent service automatically.
1. [Docker Volume Driver](#docker-volume-driver): locket can be installed as a Docker Enginer Plugin. In this mode, locket can be used as a Volume Driver, where secrets are injected directly into tmpfs-backed volumes that can be mounted by dependent containers. This allows the Docker daemon to manage the lifecycle of secrets and their injection directly, without needing a sidecar container.

## Providers

1. [1password Connect](./docs/providers/connect.md)
2. [1password Service Accounts](./docs/providers/op.md)
3. [Bitwarden Secrets Manager](./docs/providers/bws.md)
4. [Infisical](./docs/providers/infisical.md)

> [!TIP]
> Each provider has its own docker image for sidecar mode, if a slim version is preferred. The `latest` tag bundles all providers and their respective dependencies. But a provider specific tag like `locket:connect` is only about 4MB and has no extra dependencies besides what is needed for the connect provider.

## Sidecar Mode

In sidecar mode, locket runs as a separate container alongside your application container. The basic premise is:

1. Move your sensitive data to a dedicated secret manager ([Supported Providers](#providers))
1. Adjust your config files to carry *secret references* instead of raw sensitive data, which are safe to commit directly to revision control (i.e `{{ op://vault/keys/privatekey?ssh-format=openssh }}`)
1. Configure locket to use your secrets provider, or just use the docker image tag for your provider.
1. Mount your templates containing secret references for locket to read, i.e. `./templates:/templates:ro`, and mount an output directory for the secrets to be placed (usually a named tmpfs volume, or some secure location) `secrets-store:/run/secrets/locket`
1. Finally, map the template->output for each required mapping. You can map arbitrarily many directories->directories or files->files. `--map /templates:/run/secrets/locket`

Your secrets will all be injected according to the provided configuration, and any dependant applications will have materialized secrets available.

> [!TIP] 
> By default, locket will also *watch* for changes to your secret reference files, and will reflect those changes immediately to the configured output. So if you have an application which supports a dynamic config file with hot-reloading, you can manage this with locket directly without downtime. If you dont want files watched, simply use `--mode=park` to inject once and then hang out (to keep the process alive for healthchecks). Or use `--mode=one-shot` to do a single inject and exit.

A full configuration reference for all available options is provided in [`docs/inject.md`](./docs/inject.md)

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
        - "--op-token=file:/run/secrets/op_token"
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
        connect-token: file:/etc/connect/token
        connect-host: $OP_CONNECT_HOST
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
> This is behavior managed by Docker directly, and cannot be changed. So in some cases it may be necessary to get creative with the service names to ensure the secrets are namespaced as desired.

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

## Orchestration

Process orchestration is achievable via the `locket exec` command, which allows locket to act as a parent process / supervisor for a specified subcommand. It resolves secrets from your templates or `.env` files and injects them directly into the environment of a subprocess.

Optionally, the `--watch` flag can be provided so that locket will watch for changes to any
.env files provided, and restart the child process or process group. 

Full configuration reference available at [docs/exec.md](./docs/exec.md)

> [!IMPORTANT]
> locket must be installed on the host system to use this mode. Follow the [steps here](#install-prebuilt-binaries)

### Basic Usage

Simply wrap your command with `locket exec`. You can supply secrets via individual files, `.env` files, or inline arguments.

```bash
locket exec \
    --provider bws \
    --bws-token file:/path/to/token \
    --env .env \
    --env .env.override \
    --env MY_SECRET={{reference}}\
    -- docker compose up -d
```
Now, any provided env variables will be available to docker, so your compose can
reference `$MY_SECRET` for example, and it will have the resolved secret available, without ever needing it on disk or in host environment.

### Interactive Example

```sh
locket exec \
  --provider bws \
  --bws-token file:/etc/tokens/bws \
  -e MY_SECRET={{3832b656-a93b-45ad-bdfa-b267016802c3}} \
  -- python3

2025-12-14T19:29:10.126886Z  INFO Starting locket v0.14.0 `exec` service 
2025-12-14T19:29:10.708684Z  INFO resolving environment and starting process...
2025-12-14T19:29:10.709115Z  INFO batch fetching secrets count=1
2025-12-14T19:29:10.839831Z  INFO Spawning child process cmd=["python3"]
Python 3.11.2 (main, Apr 28 2025, 14:11:48) [GCC 12.2.0] on linux
Type "help", "copyright", "credits" or "license" for more information.
>>> import os
>>> os.environ["MY_SECRET"]
'ABB80C10E50A96B3CE9480D880B2CAED1A7D205A'
>>> 
```

## Docker Volume Driver

locket can run as a managed Docker Engine Plugin. This allows you to offload the lifecycle of secret injection to the Docker Daemon. Volumes created with this driver are `tmpfs` (in-memory) filesystems, ensuring secrets are never written to disk. When a volume is unmounted and no references to it remain, the secrets are automatically removed from memory.

## Installation

You can configure the plugin using environment variables or, **recommended**, by mounting a configuration directory. The full configuration reference is available at [docs/volume.md](./docs/volume.md). Note that these configurations can be placed in the global plugin config, or they can be overridden on a per-volume basis using `driver_opts`.

### File-Based Configuration

This method is more secure as it prevents tokens from appearing in `docker plugin inspect`. It also
allows you to much more easily rotate credentials without needing to restart the plugin, by simply updating the token file on the host.

1.  **Prepare Host Directory**
    Create a directory on your host (e.g., `/etc/locket`) to hold your configuration and secret tokens.

    ```bash
    sudo mkdir -p /etc/locket/tokens
    echo "your-bws-token" | sudo tee /etc/locket/tokens/bws
    sudo chmod 600 /etc/locket/tokens/bws
    ```

2.  **Create locket.toml**
    Create `/etc/locket/locket.toml`. When configuring paths in this file, remember they are relative to the *plugin's* view of the mount, which is `/etc/locket`.

    ```toml
    [volume]
    # Select the default provider. This can be overridden per volume using driver_opts.
    provider = "bws"

    # Default settings for providers
    bws-token = "file:/etc/locket/tokens/bws"
    # Configure defaults for other providers if needed.
    connect-host = "https://connect.example.com"
    connect-token = "file:/etc/locket/tokens/connect"

    # Optional: Set global defaults for all volumes created. Can also be overridden per volume.
    user = "1000:1000"
    log-level = "info"
    ```

3.  **Install the Plugin**
    Install the plugin and map your host directory to the plugin's config source.

    ```bash
    docker plugin install bpbradley/locket:plugin \
      --alias locket \
      config.source=/etc/locket
    ```

### Environment Variable Based Configuration

Useful for quick testing or simple setups.

```bash
docker plugin install bpbradley/locket:plugin \
  --alias locket \
  SECRETS_PROVIDER=op-connect \
  OP_CONNECT_HOST=https://connect.example.com \
  OP_CONNECT_TOKEN="your-token"
```

> [!WARNING]
> This method will expose your tokens in `docker plugin inspect`. Use file based configuration instead to avoid this.

### Example usage

```yaml
---
name: volume-demo
services:
    demo:
        user: 1000:1000
        image: busybox
        command:
            - "sh"
            - "-c"
            - "cat /run/secrets/locket/template && echo && sleep 30"
        volumes:
            - locket-volume:/run/secrets/locket:ro
volumes:
    locket-volume:
        driver: locket
        driver_opts:
            # Can set provider options here, or leave empty if they were set in default config.
            provider: op
            op-token: file:/etc/locket/tokens/op
            user: 1000:1000 # Make sure the container has permissions to access the volume
            mode: 0700 # Sets permissions for the mounted tmpfs volume on the host
            secret.template: "{{ op://Mordin/TestKey/private key }}"
```

### Limitations

#### File-Based Templates

While locket generally supports loading templates from files (e.g., `-o secret.config="/path/to/template.yaml"`), this is not going to be easy to leverage when running as a managed Docker Plugin (`docker plugin install ...`)

Managed plugins run in an isolated rootfs and do not have access to the host filesystem. Therefore, they cannot read template files residing on the host without placing the files somewhere the plugin can access them, (such as `/etc/locket` if following the recommended installation).

If you require file-backed templates (and the ability to watch them for changes), you must run locket as a standalone binary on the host (ideally managed via systemd, in a manner described [here](https://docs.docker.com/engine/extend/plugin_api/#plugin-lifecycle)) rather than as a managed Docker Plugin. When running as a native process, locket volume has full access to the host filesystem to read templates and watch them for changes. You will still need to specify them by absolute path though.

Example running directly on host via systemd

```ini
[Unit]
Description=Locket Docker Volume Plugin
Documentation=https://github.com/bpbradley/locket
# Start before Docker so that volumes are resolvable immediately on boot
Before=docker.service
After=network.target

[Service]
Type=simple
# Ensure the binary can be invoked directly, or provide the full path to the binary here
ExecStart=locket volume --config /etc/locket/locket.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Refer to [these instructions](#install-prebuilt-binaries) for installing the locket binary on the host.

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

1. **Templating Engine**: Adding attributes to the secret reference which can transform secrets before injection. For example `{{ secret_reference | base64 }}` to encode the secret as base64, or `{{ secret_reference | totp }}` to interpret the secret as a totp code.
1. **Swarm Operator**: Native integration for Docker Swarm secrets.
