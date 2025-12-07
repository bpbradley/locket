[Return to Index](./CONFIGURATION.md)

## `locket healthcheck`

Checks the health of the sidecar agent, determined by the state of materialized secrets. Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--status-file` | `LOCKET_STATUS_FILE` | `/tmp/.locket/ready` | Status file path used for healthchecks |
