# locket 0.14.0 -- Configuration Reference
## Commands

- [`run`](./run.md) - Start the secret sidecar agent.
All secrets will be collected and materialized according to configuration.

Example:

```sh
locket run --provider bws --bws-token-file /path/to/token \
    --secret=/path/to/secrets.yaml \
    --secret=key=@key.pem \
    --map /templates=/run/secrets/locket
```
- [`exec`](./exec.md) - Execute a command with secrets injected into the process environment.

Example:

```sh
locket exec --provider bws --bws-token-file /path/to/token \
    -e locket.env -e OVERRIDE={{ reference }} \
    -- docker compose up -d
```
- [`healthcheck`](./healthcheck.md) - Materialize secrets from environment or templates
- [`compose`](./compose.md) - Materialize secrets from environment or templates
