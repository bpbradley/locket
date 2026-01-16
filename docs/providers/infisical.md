# Infisical Provider

This provider is based on the [Infisical](https://infisical.com/) secret management platform.
It uses the [Infisical API](https://infisical.com/docs/api-reference/overview/introduction) to fetch secrets and manage authentication using Universal Auth.

## Reference syntax.

Infisical does not have a native secret reference syntax like other providers. It is fundamentally
unique in how it organizes secrets. Secrets must be part of a project and an environment, and they will have a path (either at the root of the project, or nested internally). They may also be a shared secret, or a personal one. So in order to make secret referencing work within locket, we define a custom URI scheme:

`infisical:///<secret-key>?env=<env-slug>&path=</path/to/folder>&project_id=<project-uuid>&type=<secret-type[shared | personal]>`

* The URI prefix is used to disambiguate from other providers and to easily identify Infisical secrets within templates.
* The secret key is required and is encoded in the path component.
* The environment slug, path, project ID, and secret type are optional query parameters, which override defaults (defaults set in [configuration](../inject.md#infisical-secrets-provider))

## Setup

> [!TIP]
> Below are steps to create a Machine Identity on your Infisical Organization, which makes it simpler to manage access for locket if it needs access to multiple projects. If you only need access to a single project, you can create a Machine Identity scoped to that project instead.

1. [Create an Infisical Account](https://app.infisical.com/signup)
1. Create a project, and add secrets to it.
1. Create a Machine Identity for locket. Navigate to Organization > Access Control > Machine Identities. Select `Create Organization Machine Identity`. Give it a name, and you can assign the permissions to `No Access`. 
1. Once redirected, in the Universal Auth tab, select `Add Client Secret`. Give it a name, and any TTL or usage limits as needed. Select `Create`
1. Take note of the `Client Secret` and keep it in a safe location. It will not be shown again.
1. Make sure to associate this `Client Secret` with the `Client ID` of the Universal Auth instance, as both are needed to configure locket for authentication.
1. Add any projects that you want locket to have access to here.

[Here](../inject.md#infisical-secrets-provider) is the reference configuration for locket using Infisical

```sh
locket inject --provider infisical \
  --infisical-client-secret file:/path/to/token \
  --infisical-client-id c74d3ea3-d189-43f0-96bb-649fa27bee30 \
  --infisical-default-environment dev \
  --infisical-default-project-id 6ca04a90-e171-41c2-b838-fa0d951822e3 \
  --out /run/secrets/locket \
  --secret name={{infisical:///SECRET?env=prod&path=/path/to/folder}}
  --secret /path/to/secrets.yaml \
  --secret auth_key=@key.pem \
  --map ./tpl:/run/secrets/locket/mapped 
```

## Example Sidecar Configuration

```yaml
services:
  locket:
    image: ghcr.io/bpbradley/locket:infisical
    user: "1000:1000"
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    container_name: locket-infisical
    secrets:
      - infisical_secret
    volumes:
      - ./templates:/templates:ro
      - out-infisical:/run/secrets/locket
    command: # Or use environment variables/TOML
      - "--infisical-client-secret=file:/run/secrets/infisical_secret"
      - "--infisical-client-id=c74d3ea3-d189-43f0-96bb-649fa27bee30"
      - "--infisical-default-environment=prod"
      - "--infisical-default-project-id=6ca04a90-e171-41c2-b838-fa0d951822e3"
secrets:
  connect_token:
    file: /etc/tokens/infisical
volumes:
  out-infisical: { driver: local, driver_opts: { type: tmpfs, device: tmpfs, o: "uid=1000,gid=1000,mode=0700" } }
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
        provider: infisical
        infisical-client-secret: file:/etc/tokens/infisical
        infisical-client-id: "c74d3ea3-d189-43f0-96bb-649fa27bee30"
        infisical-default-project-id: "6ca04a90-e171-41c2-b838-fa0d951822e3"
        infisical-default-environment: "dev"
        env:
            - TEXT={{infisical:///SECRET}}
            - SECRET={{infisical:///SECRET?env=prod}}
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
