[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket volume`

Run as a Docker Volume Plugin

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--config` | `LOCKET_CONFIG` |  | Path to configuration files<br><br>Can be specified multiple times to layer multiple files. Each file is loaded in the order specified, with later files overriding earlier ones. |
| `--socket` | `LOCKET_PLUGIN_SOCKET` |  | Path to the listening socket.<br><br>Docker plugins usually listen on /run/docker/plugins/<name>.sock |
| `--state-dir` | `LOCKET_PLUGIN_STATE_DIR` |  |  |
| `--runtime-dir` | `LOCKET_PLUGIN_RUNTIME_DIR` |  |  |
| `--log-format` | `LOCKET_LOG_FORMAT` |  | Log format <br><br> **Choices:**<br>- `text`: Plain text log format<br>- `json`: JSON log format<br>- `compose`: Special format for Docker Compose Provider specification |
| `--log-level` | `LOCKET_LOG_LEVEL` |  | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br><br> **Choices:**<br>- `op`: 1Password Service Account<br>- `op-connect`: 1Password Connect Provider<br>- `bws`: Bitwarden Secrets Provider<br>- `infisical`: Infisical Secrets Provider |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op-token` | `OP_SERVICE_ACCOUNT_TOKEN` |  | 1Password Service Account Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--op-config-dir` | `OP_CONFIG_DIR` |  | Optional: Path to 1Password config directory<br><br>Defaults to standard op config locations if not provided, e.g. `$XDG_CONFIG_HOME/op` |
### 1Password Connect

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--connect-host` | `OP_CONNECT_HOST` |  | 1Password Connect Host HTTP(S) URL |
| `--connect-token` | `OP_CONNECT_TOKEN` |  | 1Password Connect Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--connect-max-concurrent` | `OP_CONNECT_MAX_CONCURRENT` |  | Maximum allowed concurrent requests to Connect API |
### Bitwarden Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bws-api-url` | `BWS_API_URL` |  | Bitwarden API URL |
| `--bws-identity-url` | `BWS_IDENTITY_URL` |  | Bitwarden Identity URL |
| `--bws-max-concurrent` | `BWS_MAX_CONCURRENT` |  | Maximum number of concurrent requests to Bitwarden Secrets Manager |
| `--bws-user-agent` | `BWS_USER_AGENT` |  | BWS User Agent |
| `--bws-token` | `BWS_MACHINE_TOKEN` |  | Bitwarden Machine Token<br><br>Either provide the token directly or via a file with `file:` prefix |
### Infisical Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--infisical-url` | `INFISICAL_URL` |  | The URL of the Infisical instance to connect to |
| `--infisical-client-secret` | `INFISICAL_CLIENT_SECRET` |  | The client secret for Universal Auth to authenticate with Infisical.<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--infisical-client-id` | `INFISICAL_CLIENT_ID` |  | The client ID for Universal Auth to authenticate with Infisical |
| `--infisical-default-environment` | `INFISICAL_DEFAULT_ENVIRONMENT` |  | The default environment slug to use when one is not specified |
| `--infisical-default-project-id` | `INFISICAL_DEFAULT_PROJECT_ID` |  | The default project ID to use when one is not specified |
| `--infisical-default-path` | `INFISICAL_DEFAULT_PATH` |  | The default path to use when one is not specified |
| `--infisical-default-secret-type` | `INFISICAL_DEFAULT_SECRET_TYPE` |  | The default secret type to use when one is not specified <br><br> **Choices:**<br>- `shared`<br>- `personal` |
| `--infisical-max-concurrent` | `INFISICAL_MAX_CONCURRENT` |  | Maximum allowed concurrent requests to Infisical API |
