[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket volume`

Run as a Docker Volume Plugin

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--config` | `LOCKET_CONFIG` |  | Path to configuration files<br><br>Can be specified multiple times to layer multiple files. Each file is loaded in the order specified, with later files overriding earlier ones. |
| `--secrets` | `LOCKET_VOLUME_DEFAULT_SECRETS` |  | Default secrets to mount into the volume<br><br>These will typically be specified in driver_opts for volume. However, default secrets can be provided via CLI/ENV which would be available to all volumes by default. |
| `--user` | `LOCKET_FILE_OWNER` |  | Owner of the file/dir<br><br>Defaults to the running user/group. The running user must have write permissions on the directory to change the owner. |
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br><br> **Choices:**<br>- `op`: 1Password Service Account<br>- `op-connect`: 1Password Connect Provider<br>- `bws`: Bitwarden Secrets Provider<br>- `infisical`: Infisical Secrets Provider |
| `--socket` | `LOCKET_PLUGIN_SOCKET` | `/run/docker/plugins/locket.sock` | Path to the listening socket |
| `--state-dir` | `LOCKET_PLUGIN_STATE_DIR` | `/var/lib/locket` |  |
| `--runtime-dir` | `LOCKET_PLUGIN_RUNTIME_DIR` | `/run/locket/volumes` |  |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br><br> **Choices:**<br>- `text`: Plain text log format<br>- `json`: JSON log format<br>- `compose`: Special format for Docker Compose Provider specification |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |
| `--watch` | `LOCKET_VOLUME_DEFAULT_WATCH` | `false` | Default behavior for file watching.<br><br>If set to true, the volume will watch for changes in the secrets and update the files accordingly. <br><br> **Choices:**<br>- `true`<br>- `false` |
| `--inject-failure-policy` | `LOCKET_VOLUME_DEFAULT_INJECT_POLICY` | `passthrough` | Default policy for handling failures when errors are encountered <br><br> **Choices:**<br>- `error`: Failures are treated as errors and will abort the process<br>- `passthrough`: On failure, copy the unmodified secret to destination<br>- `ignore`: On failure, ignore the secret and log a warning |
| `--max-file-size` | `LOCKET_VOLUME_DEFAULT_MAX_FILE_SIZE` | `10M` | Default maximum size of individual secret files |
| `--file-mode` | `LOCKET_FILE_MODE` | `0600` | File permission mode |
| `--dir-mode` | `LOCKET_DIR_MODE` | `0700` | Directory permission mode |
| `--size` | `LOCKET_VOLUME_DEFAULT_MOUNT_SIZE` | `10M` | Default size of the in-memory filesystem |
| `--mode` | `LOCKET_VOLUME_DEFAULT_MOUNT_MODE` | `0700` | Default file mode for the mounted filesystem |
| `--flags` | `LOCKET_VOLUME_DEFAULT_MOUNT_FLAGS` | `rw,noexec,nosuid,nodev` | Default mount flags for the in-memory filesystem |
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
| `--connect-max-concurrent` | `OP_CONNECT_MAX_CONCURRENT` | `20` | Maximum allowed concurrent requests to Connect API |
### Bitwarden Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bws-token` | `BWS_MACHINE_TOKEN` |  | Bitwarden Machine Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--bws-api-url` | `BWS_API_URL` | `https://api.bitwarden.com` | Bitwarden API URL |
| `--bws-identity-url` | `BWS_IDENTITY_URL` | `https://identity.bitwarden.com` | Bitwarden Identity URL |
| `--bws-max-concurrent` | `BWS_MAX_CONCURRENT` | `20` | Maximum number of concurrent requests to Bitwarden Secrets Manager |
| `--bws-user-agent` | `BWS_USER_AGENT` | `locket` | BWS User Agent |
### Infisical Secrets Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--infisical-client-secret` | `INFISICAL_CLIENT_SECRET` |  | The client secret for Universal Auth to authenticate with Infisical.<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--infisical-client-id` | `INFISICAL_CLIENT_ID` |  | The client ID for Universal Auth to authenticate with Infisical |
| `--infisical-default-environment` | `INFISICAL_DEFAULT_ENVIRONMENT` |  | The default environment slug to use when one is not specified |
| `--infisical-default-project-id` | `INFISICAL_DEFAULT_PROJECT_ID` |  | The default project ID to use when one is not specified |
| `--infisical-url` | `INFISICAL_URL` | `https://us.infisical.com` | The URL of the Infisical instance to connect to |
| `--infisical-default-path` | `INFISICAL_DEFAULT_PATH` | `/` | The default path to use when one is not specified |
| `--infisical-default-secret-type` | `INFISICAL_DEFAULT_SECRET_TYPE` | `shared` | The default secret type to use when one is not specified <br><br> **Choices:**<br>- `shared`<br>- `personal` |
| `--infisical-max-concurrent` | `INFISICAL_MAX_CONCURRENT` | `20` | Maximum allowed concurrent requests to Infisical API |

## TOML Reference

> [!TIP]
> Settings can be provided via config.toml as well, using the --config option.
> Provided is the reference configuration in TOML format

```toml
# Path to the listening socket
socket = "/run/docker/plugins/locket.sock"

state-dir = "/var/lib/locket"

runtime-dir = "/run/locket/volumes"

# Log format
log-format = "text"

# Log level
log-level = "info"

# Default secrets to mount into the volume
secrets = []

# Default behavior for file watching
watch = false

# Default policy for handling failures when errors are encountered
inject-failure-policy = "passthrough"

# Default maximum size of individual secret files
max-file-size = "10M"

# File permission mode
file-mode = "0600"

# Directory permission mode
dir-mode = "0700"

# Owner of the file/dir
# user = ...

# Default size of the in-memory filesystem
size = "10M"

# Default file mode for the mounted filesystem
mode = "0700"

# Default mount flags for the in-memory filesystem
flags = "rw,noexec,nosuid,nodev"

# Secrets provider backend to use
# provider = ...

# 1Password Service Account Token
# op-token = ...

# Optional: Path to 1Password config directory
# op-config-dir = ...

# 1Password Connect Host HTTP(S) URL
# connect-host = ...

# 1Password Connect Token
# connect-token = ...

# Maximum allowed concurrent requests to Connect API
connect-max-concurrent = 20

# Bitwarden API URL
bws-api-url = "https://api.bitwarden.com/"

# Bitwarden Identity URL
bws-identity-url = "https://identity.bitwarden.com/"

# Maximum number of concurrent requests to Bitwarden Secrets Manager
bws-max-concurrent = 20

# BWS User Agent
bws-user-agent = "locket"

# Bitwarden Machine Token
# bws-token = ...

# The URL of the Infisical instance to connect to
infisical-url = "https://us.infisical.com/"

# The client secret for Universal Auth to authenticate with Infisical
# infisical-client-secret = ...

# The client ID for Universal Auth to authenticate with Infisical
# infisical-client-id = ...

# The default environment slug to use when one is not specified
# infisical-default-environment = ...

# The default project ID to use when one is not specified
# infisical-default-project-id = ...

# The default path to use when one is not specified
infisical-default-path = "/"

# The default secret type to use when one is not specified
infisical-default-secret-type = "shared"

# Maximum allowed concurrent requests to Infisical API
infisical-max-concurrent = 20

```
