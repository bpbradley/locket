# locket 0.14.0 -- Configuration Reference
## Commands

- [`run`](./run.md) - Start the secret sidecar agent. All secrets will be collected and materialized according to configuration. Example: `locket run --provider bws --bws-token-file /path/to/token --secret=/path/to/secrets.yaml --secret=key=@key.pem --map /templates=/run/secrets/locket`
- [`exec`](./exec.md) - Execute a command with secrets injected into the process environment. Example: `locket exec --provider bws --bws-token-file /path/to/token -e locket.env -e OVERRIDE={{ reference }} -- docker compose up -d`
- [`healthcheck`](./healthcheck.md) - Checks the health of the sidecar agent, determined by the state of materialized secrets. Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code
- [`compose`](./compose.md) - Docker Compose provider API
