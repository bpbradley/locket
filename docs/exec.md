[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket exec`

Execute a command with secrets injected into the process environment.
and optionally materialize secrets from template files.

Example:

```sh
locket exec --provider bws --bws-token=file:/path/to/token \
    -e locket.env -e OVERRIDE={{ reference }}
    --map ./tpl/config:/app/config \
    -- docker compose up -d
```

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--watch` | `LOCKET_EXEC_WATCH` | `false` | Watch mode will monitor for changes to .env files and restart the command if changes are detected <br><br> **Choices:**<br>- `true`<br>- `false` |
| `--interactive` | `LOCKET_EXEC_INTERACTIVE` |  | Run the command in interactive mode, attaching stdin/stdout/stderr.<br><br>If not specified, defaults to true in non-watch mode and false in watch mode. <br><br> **Choices:**<br>- `true`<br>- `false` |
| `--env-file` | `LOCKET_ENV_FILE` |  | Files containing environment variables which may contain secret references |
| `--env` | `LOCKET_ENV` |  | Environment variable overrides which may contain secret references |
| `--map` | `SECRET_MAP` |  | Mapping of source paths to destination paths.<br><br>Maps sources (holding secret templates) to destination paths (where secrets are materialized) in the form `SRC:DST` or `SRC=DST`.<br><br>Multiple mappings can be provided, separated by commas, or supplied multiple times as arguments.<br><br>Example: `--map /templates:/run/secrets/app`<br><br>**CLI Default:** No mappings <br>**Docker Default:** `/templates:/run/secrets/locket` |
| `--secret` | `LOCKET_SECRETS` |  | Additional secret values specified as LABEL=SECRET_TEMPLATE<br><br>Multiple values can be provided, separated by commas. Or supplied multiple times as arguments.<br><br>Loading from file is supported via `LABEL=@/path/to/file`.<br><br>Example:<br><br>```sh --secret db_password={{op://..}} --secret api_key={{op://..}} ``` |
| `--out` | `DEFAULT_SECRET_DIR` | `/run/secrets/locket` | Directory where secret values (literals) are materialized |
| `--inject-policy` | `INJECT_POLICY` | `copy-unmodified` | Policy for handling injection failures <br><br> **Choices:**<br>- `error`: Failures are treated as errors and will abort the process<br>- `copy-unmodified`: On failure, copy the unmodified secret to destination<br>- `ignore`: On failure, ignore the secret and log a warning |
| `--max-file-size` | `MAX_FILE_SIZE` | `10M` | Maximum allowable size for a template file. Files larger than this will be rejected.<br><br>Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes). |
| `--file-mode` | `LOCKET_FILE_MODE` | `600` | File permission mode |
| `--dir-mode` | `LOCKET_DIR_MODE` | `700` | Directory permission mode |
| `--timeout` | `LOCKET_EXEC_TIMEOUT` | `30s` | Timeout duration for process termination signals. Unitless numbers are interpreted as seconds |
| `--debounce` | `WATCH_DEBOUNCE` | `500ms` | Debounce duration for filesystem events in watch mode.<br><br>Events occurring within this duration will be coalesced into a single update so as to not overwhelm the secrets manager with rapid successive updates from filesystem noise.<br><br>Handles human-readable strings like "100ms", "2s", etc. Unitless numbers are interpreted as milliseconds. |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br><br> **Choices:**<br>- `text`: Plain text log format<br>- `json`: JSON log format<br>- `compose`: Special format for Docker Compose Provider specification |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |
| `<cmd>` |  |  | Command to execute with secrets injected into environment<br><br>Must be the last argument(s), following a `--` separator.<br><br>Example: `locket exec -e locket.env -- docker compose up -d` |
### Provider Configuration

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br><br> **Choices:**<br>- `op`: 1Password Service Account<br>- `op-connect`: 1Password Connect Provider<br>- `bws`: Bitwarden Secrets Provider |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op.token` | `OP_SERVICE_ACCOUNT_TOKEN` |  | 1Password Service Account Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--op.config-dir` | `OP_CONFIG_DIR` |  | Optional: Path to 1Password config directory<br><br>Defaults to standard op config locations if not provided, e.g. `$XDG_CONFIG_HOME/op` |
### 1Password Connect

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--connect.host` | `OP_CONNECT_HOST` |  | 1Password Connect Host HTTP(S) URL |
| `--connect.token` | `OP_CONNECT_TOKEN` |  | 1Password Connect Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--connect.max-concurrent` | `OP_CONNECT_MAX_CONCURRENT` | `20` | Maximum allowed concurrent requests to Connect API |
### Bitwarden Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bws.api` | `BWS_API_URL` | `https://api.bitwarden.com` | Bitwarden API URL |
| `--bws.identity` | `BWS_IDENTITY_URL` | `https://identity.bitwarden.com` | Bitwarden Identity URL |
| `--bws.max-concurrent` | `BWS_MAX_CONCURRENT` | `20` | Maximum number of concurrent requests to Bitwarden Secrets Manager |
| `--bws.user-agent` | `BWS_USER_AGENT` | `locket` | BWS User Agent |
| `--bws.token` | `BWS_MACHINE_TOKEN` |  | Bitwarden Machine Token<br><br>Either provide the token directly or via a file with `file:` prefix |
