# Locket

> *A flexible secrets management agent to keep your secrets safe*

1. [Overview](#overview)
1. [Quick Start](#overview)
1. [Full Configuration](./docs/run.md)
1. [Permissions](#permissions)
1. [Examples](#example-hot-reloading-traefik-configurations-with-secrets)
1. [Supported Providers](#supported-providers)
1. [Roadmap](#roadmap)


## Overview
Locket is a small CLI tool, designed to be run as a sidecar process for your applications
which might carry sensitive data in configuration files. You might want these files off disk completely in tmpfs, or simply somewhere out of revision control.

The basic premise is that you would move these secrets to a dedicated secret manager, and adjust your config files to carry **secret references** instead of raw secrets, which are
safe to commit directly to revision control.

Then, you simply configure locket for your secrets provider, and provide your safe, secret-free files as templates. Locket will find all secret references, collect their actual value from your secrets provider, and inject them to your provided output destination.

By default, locket will also *watch* for changes to your secret reference files, and will
reflect those changes immediately to the actual output. So if you have an application which
supports a dynamic config file with hot-reloading, you can manage this with locket directly.

## Quick Start

A full configuration reference for all available options is provided in [`docs/run.md`](./docs/run.md)

We can use locket as a small sidecar service to safely inject secrets to tmpfs
before our primary service starts.

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:latest
    user: "65532:65532" # The default user is 65532:65532 (nonroot) when not specified
    command:
        - "--provider=op"
        - "--token-file=/run/secrets/op_token"
        - "--map=/templates:/run/secrets" # Supports multiple maps, if needed.
        - "--secret=db_pass={{op://vaults/db_pass}}"
        - "--secret=db_host={{op://vaults/db_host}}"
    # All configurations support command config, or env, or a mixture
    # environment:
    #  SECRETS_PROVIDER: "op"
    #  OP_SERVICE_ACCOUNT_TOKEN_FILE: "/run/secrets/op_token"
    #  SECRET_MAP: "/templates:/run/secrets"
    #  SECRET_VALUES: "db_pass={{op://vaults/db_pass}};db_host={{op://vaults/db_host}}"
    secrets:
      - op_token
    volumes:
        # Mount in your actual secret templates, with secret references
      - ./config/templates:/templates:ro
        # Mount in your output directory, where you want secrets materialized
      - secrets-store:/run/secrets
  app:
    image: my-app:latest
    volumes:
      # Mount the shared volume wherever you want the secrets in the container
      - secrets-store:/run/secrets:ro
    # We can force the application to wait until locket is healthy, meaning all
    # secrets were successfully injected.
    environment:
        # We can directly reference the materialized secrets
        DB_PASSWORD_FILE: /run/secrets/db_pass
        DB_HOST_FILE: /run/secrets/db_host
    depends_on:
        locket:
            condition: healthy
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

Because locket was designed to run distroless and non-root by default, it is important
to understand some pitfalls with file permissions which might arise.

1. The locket image is designed distroless and non-root, running as uid=65532 and gid=65532
with a user and group `nonroot` by default. This is a standard set by Googles distroless
images which was adopted here.
1. Template files, output directories, and auth tokens will need to have permissions for this user to read them. If docker volumes, this will usually "just work". But if not, the uid,gid,
and mode can be set for tmpfs volumes directly. i.e. `o: "uid=1000,gid=1000,mode=700"` in `driver_opts`.
1. The container can be run as root, or any arbitrary non-root user besides 65532 using the `user` directive in docker, however some extra steps are needed (at the moment) to support this use case.

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

Right now, only 1password is supported, but the architecture of locket put significant emphasis on making sure this would be easy to expand later.

### Using 1password provider with non-default user

Right now, using 1password with non-default user is possible, but annoying due to some
restrictions with the `op` cli tool being very aggressive in its security posture surrounding
its config files. Basically, `op` requires that a config file exists (where it places some details about the credentialed user/device), and it requires that folder be owned by the user process, that it not have permissions broader than 0700, and importantly, that the user is resolvable via /etc/passwd. 

The default `65532:65532` is resolved via /etc/passwd to `nonroot` and so all of this is fine.
But a different user than this must supply an /etc/password with that uid resolving to some entry (it doesn't matter what). So in this case, you must mount something in like

```
volumes:
    /etc/passwd:/etc/passwd:ro
    op-data:/config
```

> [!NOTE]
> This will be fixed by v1.0.0 when the `op` dependency is removed.

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

