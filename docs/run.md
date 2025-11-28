[Return](./CONFIGURATION.md)

# locket run

Start the secret sidecar agent. All secrets will be collected and materialized according to configuration

### Arguments

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--mode` | `RUN_MODE` | `watch` | Mode of operation<br><br> **Options:**<br> - `one-shot`: Collect and materialize all secrets once and then exit<br> - `watch`: Continuously watch for changes on configured templates and update secrets as needed<br> - `park`: Run once and then park to keep the process alive |
| `--status-file` | `STATUS_FILE` | `/tmp/.locket/ready` | Status file path used for healthchecks |
| `--map` | `SECRET_MAP` | `/templates:/run/secrets` | Mapping of source paths (holding secret templates) to destination paths (where secrets are materialized and reflected) in the form SRC:DST or SRC=DST. Multiple mappings can be provided, separated by commas, or supplied multiple times as arguments |
| `--out` | `VALUE_OUTPUT_DIR` | `/run/secrets` | Directory where secret values (literals) are materialized |
| `--inject-policy` | `INJECT_POLICY` | `copy-unmodified` | Policy for handling injection failures <br><br> **Options:** `error`, `copy-unmodified`, `ignore` |
| `--env-value-prefix` | `VALUE_PREFIX` | `secret_` | Environment variables prefixed with this string will be treated as secret values |
| `--secret` | `SECRET_VALUE` | *None* | Additional secret values specified as LABEL=SECRET_TEMPLATE |
| `--file-mode` | *None* | `600` | File permission mode |
| `--dir-mode` | *None* | `700` | Directory permission mode |
| `--log-format` | `LOG_FORMAT` | `text` | Log format <br><br> **Options:** `text`, `json` |
| `--log-level` | `LOG_LEVEL` | `info` | Log level <br><br> **Options:** `trace`, `debug`, `info`, `warn`, `error` |
### Provider Configuration

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--provider` | `SECRETS_PROVIDER` | *None* | Secrets provider <br><br> **Options:** `op` |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--token` | `OP_SERVICE_ACCOUNT_TOKEN` | *None* | 1Password (op) service account token |
| `--token-file` | `OP_SERVICE_ACCOUNT_TOKEN_FILE` | *None* | Path to file containing the service account token |
| `--config-dir` | `OP_CONFIG_DIR` | *None* | Path to 1Password (op) config directory |
