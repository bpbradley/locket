[Return](./CONFIGURATION.md)

# locket run

Run Secret Sidecar

### Arguments

| Option | Env Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| Mode <br> `--mode` | `RUN_MODE` | `watch` | Run mode<br><br> **Options:**<br> - `one-shot`: Run once and exit<br> - `watch`: Watch for changes and re-apply<br> - `park`: Run once and then park to keep the process alive |
| Path <br> `--status-file` | `STATUS_FILE` | `/tmp/.locket/ready` | Status file path |
| Mapping <br> `--map` | `SECRET_MAP` | `/templates:/run/secrets` | Mapping of source paths (holding secret templates) to destination paths (where secrets are materialized and reflected) |
| Value Dir <br> `--out` | `VALUE_OUTPUT_DIR` | `/run/secrets` | Directory where secret values (literals) are materialized |
| Policy <br> `--inject-policy` | `INJECT_POLICY` | `copy-unmodified` | Policy for handling injection failures <br><br> **Options:** `error`, `copy-unmodified`, `ignore` |
| Env Value Prefix <br> `--env-value-prefix` | `VALUE_PREFIX` | `secret_` | Environment variables prefixed with this string will be treated as secret values |
| Values <br> `--secret` | `SECRET_VALUE` | *None* | Additional secret values specified as LABEL=SECRET_TEMPLATE |
| File Mode <br> `--file-mode` | *None* | `600` | File permission mode |
| Dir Mode <br> `--dir-mode` | *None* | `700` | Directory permission mode |
| Log Format <br> `--log-format` | `LOG_FORMAT` | `text` | Log format <br><br> **Options:** `text`, `json` |
| Log Level <br> `--log-level` | `LOG_LEVEL` | `info` | Log level <br><br> **Options:** `trace`, `debug`, `info`, `warn`, `error` |
### Provider Configuration

| Option | Env Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| Kind <br> `--provider` | `SECRETS_PROVIDER` | *None* | Secrets provider <br><br> **Options:** `op` |
### 1Password (op)

| Option | Env Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| Token <br> `--token` | `OP_SERVICE_ACCOUNT_TOKEN` | *None* | 1Password (op) service account token |
| Token File <br> `--token-file` | `OP_SERVICE_ACCOUNT_TOKEN_FILE` | *None* | Path to file containing the service account token |
| Config Dir <br> `--config-dir` | `OP_CONFIG_DIR` | *None* | Path to 1Password (op) config directory |
