[Return to Index](./CONFIGURATION.md)

# `locket run`

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

### General

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--mode` | `LOCKET_RUN_MODE` | `watch` | Mode of operation<br><br> **Options:**<br> - `one-shot`: Collect and materialize all secrets once and then exit<br> - `watch`: Continuously watch for changes on configured templates and update secrets as needed<br> - `park`: Run once and then park to keep the process alive |
| `--status-file` | `LOCKET_STATUS_FILE` | `/tmp/.locket/ready` | Status file path used for healthchecks |
| `--map` | `SECRET_MAP` | `/templates:/run/secrets` | Mapping of source paths (holding secret templates) to destination paths (where secrets are materialized and reflected) in the form `SRC:DST` or `SRC=DST`. Multiple mappings can be provided, separated by commas, or supplied multiple times as arguments. e.g. `--map /templates:/run/secrets/app --map /other_templates:/run/secrets/other` |
| `--out` | `VALUE_OUTPUT_DIR` | `/run/secrets` | Directory where secret values (literals) are materialized |
| `--inject-policy` | `INJECT_POLICY` | `copy-unmodified` | Policy for handling injection failures<br><br> **Options:**<br> - `error`: Injection failures are treated as errors and will abort the process<br> - `copy-unmodified`: On injection failure, copy the unmodified secret to destination<br> - `ignore`: On injection failure, just log a warning and proceed with the secret ignored |
| `--max-file-size` | `MAX_FILE_SIZE` | `10M` | Maximum allowable size for a template file. Files larger than this will be rejected. Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes) |
| `--env-prefix` | `VALUE_PREFIX` | `secret_` | Environment variables prefixed with this string will be treated as secret values |
| `--secret` | `SECRET_VALUE` | *None* | Additional secret values specified as LABEL=SECRET_TEMPLATE Multiple values can be provided, separated by semicolons. Or supplied multiple times as arguments. e.g. `--secret db_password={{op://vault/credentials/db_password}} --secret api_key={{op://vault/keys/api_key}}` |
| `--debounce-ms` | `WATCH_DEBOUNCE_MS` | `500` | Debounce duration in milliseconds for filesystem events. Events occurring within this duration will be coalesced into a single update so as to not overwhelm the secrets manager with rapid successive updates from filesystem noise |
| `--file-mode` | `LOCKET_FILE_MODE` | `600` | File permission mode |
| `--dir-mode` | `LOCKET_DIR_MODE` | `700` | Directory permission mode |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br><br> **Options:** `text`, `json` |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br><br> **Options:** `trace`, `debug`, `info`, `warn`, `error` |
### Provider Configuration

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--provider` | `SECRETS_PROVIDER` | *None* | Secrets provider<br><br> **Options:**<br> - `op`: 1Password Service Account<br> - `op-connect`: 1Password Connect Provider<br> - `bws`: Bitwarden Secrets Provider |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op.token` | `OP_SERVICE_ACCOUNT_TOKEN` | *None* | 1Password Service Account token |
| `--op.token-file` | `OP_SERVICE_ACCOUNT_TOKEN_FILE` | *None* | Path to file containing 1Password Service Account token |
| `--op.config-dir` | `OP_CONFIG_DIR` | *None* | Optional: Path to 1Password config directory Defaults to standard op config locations if not provided, e.g. $XDG_CONFIG_HOME/op |
### 1Password Connect

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--connect.host` | `OP_CONNECT_HOST` | *None* | 1Password Connect Host HTTP(S) URL |
| `--connect.token` | `OP_CONNECT_TOKEN` | *None* | 1Password Connect API token |
| `--connect.token-file` | `OP_CONNECT_TOKEN_FILE` | *None* | Path to file containing 1Password Connect API token |
| `--connect.max-concurrent` | `OP_CONNECT_MAX_CONCURRENT` | `20` | Maximum allowed concurrent requests to Connect API |
### Bitwarden Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bws.api` | `BWS_API_URL` | `https://api.bitwarden.com` | Bitwarden API URL |
| `--bws.identity` | `BWS_IDENTITY_URL` | `https://identity.bitwarden.com` | Bitwarden Identity URL |
| `--bws.max-concurrent` | `BWS_MAX_CONCURRENT` | `20` | Maximum number of concurrent requests to Bitwarden Secrets Manager |
| `--bws.token` | `BWS_MACHINE_TOKEN` | *None* | Bitwarden Secrets Manager machine token |
| `--bws.token-file` | `BWS_MACHINE_TOKEN_FILE` | *None* | Path to file containing Bitwarden Secrets Manager machine token |
