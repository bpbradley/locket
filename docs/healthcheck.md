[Return](./CONFIGURATION.md)

# locket healthcheck

Checks the health of the sidecar agent, determined by the state of materialized secrets. Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code

### Arguments

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--status-file` | `STATUS_FILE` | `/tmp/.locket/ready` | Status file path used for healthchecks |
