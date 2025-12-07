[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket run`

Start the secret sidecar agent. All secrets will be collected and materialized according to configuration

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--mode` | `LOCKET_RUN_MODE` | `watch` | Mode of operation <br> **Choices:** `one-shot`, `watch`, `park` |
| `--status-file` | `LOCKET_STATUS_FILE` | `/tmp/.locket/ready` | Status file path used for healthchecks |
| `--map` | `SECRET_MAP` | `/templates:/run/secrets/locket` | Mapping of source paths (holding secret templates) to destination paths (where secrets are materialized and reflected) in the form `SRC:DST` or `SRC=DST`. Multiple mappings can be provided, separated by commas, or supplied multiple times as arguments. e.g. `--map /templates:/run/secrets/locket/app --map /other_templates:/run/secrets/locket/other` |
| `--out` | `VALUE_OUTPUT_DIR` | `/run/secrets/locket` | Directory where secret values (literals) are materialized |
| `--inject-policy` | `INJECT_POLICY` | `copy-unmodified` | Policy for handling injection failures <br> **Choices:** `error`, `copy-unmodified`, `ignore` |
| `--max-file-size` | `MAX_FILE_SIZE` | `10M` | Maximum allowable size for a template file. Files larger than this will be rejected. Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes) |
| `--secret` | `LOCKET_SECRETS` |  | Additional secret values specified as LABEL=SECRET_TEMPLATE Multiple values can be provided, separated by commas. Or supplied multiple times as arguments. e.g. `--secret db_password={{op://..}} --secret api_key={{op://..}}` |
| `--debounce-ms` | `WATCH_DEBOUNCE_MS` | `500` | Debounce duration in milliseconds for filesystem events. Events occurring within this duration will be coalesced into a single update so as to not overwhelm the secrets manager with rapid successive updates from filesystem noise |
| `--file-mode` | `LOCKET_FILE_MODE` | `600` | File permission mode |
| `--dir-mode` | `LOCKET_DIR_MODE` | `700` | Directory permission mode |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br> **Choices:** `text`, `json` |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br> **Choices:** `trace`, `debug`, `info`, `warn`, `error` |
### Provider Configuration

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider <br> **Choices:** `op`, `op-connect`, `bws` |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op.token` | `OP_SERVICE_ACCOUNT_TOKEN` |  | 1Password Service Account token |
| `--op.token-file` | `OP_SERVICE_ACCOUNT_TOKEN_FILE` |  | Path to file containing 1Password Service Account token |
| `--op.config-dir` | `OP_CONFIG_DIR` |  | Optional: Path to 1Password config directory Defaults to standard op config locations if not provided, e.g. $XDG_CONFIG_HOME/op |
### 1Password Connect

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--connect.host` | `OP_CONNECT_HOST` |  | 1Password Connect Host HTTP(S) URL |
| `--connect.token` | `OP_CONNECT_TOKEN` |  | 1Password Connect API token |
| `--connect.token-file` | `OP_CONNECT_TOKEN_FILE` |  | Path to file containing 1Password Connect API token |
| `--connect.max-concurrent` | `OP_CONNECT_MAX_CONCURRENT` | `20` | Maximum allowed concurrent requests to Connect API |
### Bitwarden Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bws.api` | `BWS_API_URL` | `https://api.bitwarden.com` | Bitwarden API URL |
| `--bws.identity` | `BWS_IDENTITY_URL` | `https://identity.bitwarden.com` | Bitwarden Identity URL |
| `--bws.max-concurrent` | `BWS_MAX_CONCURRENT` | `20` | Maximum number of concurrent requests to Bitwarden Secrets Manager |
| `--bws.user-agent` | `BWS_USER_AGENT` | `locket` | BWS User Agent |
| `--bws.token` | `BWS_MACHINE_TOKEN` |  | Bitwarden Secrets Manager machine token |
| `--bws.token-file` | `BWS_MACHINE_TOKEN_FILE` |  | Path to file containing Bitwarden Secrets Manager machine token |
