[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket exec`

Execute a command with secrets injected into the process environment.

Example:

    ```sh
    locket exec --provider bws --bws-token-file /path/to/token \
        -e locket.env -e OVERRIDE={{ reference }} \
        -- docker compose up -d
    ```

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--watch` | `LOCKET_EXEC_WATCH` | `false` | Watch mode will monitor for changes to .env files and restart the command if changes are detected <br> **Choices:** `true`, `false` |
| `--interactive` | `LOCKET_EXEC_INTERACTIVE` |  | Run the command in interactive mode, attaching stdin/stdout/stderr. If not specified, defaults to true in non-watch mode and false in watch mode <br> **Choices:** `true`, `false` |
| `--env-file` | `LOCKET_ENV_FILE` |  | Files containing environment variables which may contain secret references |
| `--env` | `LOCKET_ENV` |  | Environment variable overrides which may contain secret references |
| `--timeout` | `LOCKET_EXEC_TIMEOUT` | `30s` | Timeout duration for process termination signals. Unitless numbers are interpreted as seconds |
| `--debounce` | `WATCH_DEBOUNCE` | `500ms` | Debounce duration for filesystem events in watch mode. Events occurring within this duration will be coalesced into a single update so as to not overwhelm the secrets manager with rapid successive updates from filesystem noise. Handles human-readable strings like "100ms", "2s", etc. Unitless numbers are interpreted as milliseconds |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br> **Choices:** `text`, `json` |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br> **Choices:** `trace`, `debug`, `info`, `warn`, `error` |
| `<cmd>` |  |  | Command to execute with secrets injected into environment Must be the last argument(s), following a `--` separator. Example: locket exec -e locket.env -- docker compose up -d |
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
