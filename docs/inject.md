[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket inject`

Inject secrets from secret references into files and directories.

Example:

```sh
locket inject --provider bws --bws-token=file:/path/to/token \ # Select the BWS provider
    --out /run/secrets/locket \ # Default output directory
    --secret=/path/to/secrets.yaml \ # An anonymous secret file, placed in `/run/secrets/locket/secrets.yaml`
    --secret=auth_key=@key.pem \ # A named secret file, placed in `/run/secrets/locket/auth_key`
    --map ./tpl:/run/secrets/locket/mapped \ # Maps all templates in `./tpl/` directory to secrets in `/run/secrets/locket/mapped`
```

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--config` | `LOCKET_CONFIG` |  | Path to configuration files<br><br>Can be specified multiple times to layer multiple files. Each file is loaded in the order specified, with later files overriding earlier ones. |
| `--status-file` | `LOCKET_STATUS_FILE` |  | Status file path used for healthchecks.<br><br>If not provided, no status file is created.<br><br>**Docker Default:** `/dev/shm/locket/ready` |
| `--map` | `SECRET_MAP` |  | Mapping of source paths to destination paths.<br><br>Maps sources (holding secret templates) to destination paths (where secrets are materialized) in the form `SRC:DST` or `SRC=DST`.<br><br>Multiple mappings can be provided, separated by commas, or supplied multiple times as arguments.<br><br>Example: `--map /templates:/run/secrets/app`<br><br>**CLI Default:** No mappings <br>**Docker Default:** `/templates:/run/secrets/locket` |
| `--secrets` | `LOCKET_SECRETS` |  | Additional secret values specified as LABEL=SECRET_TEMPLATE<br><br>Multiple values can be provided, separated by commas. Or supplied multiple times as arguments.<br><br>Loading from file is supported via `LABEL=@/path/to/file`.<br><br>Example:<br><br>```sh --secret db_password={{op://..}} --secret api_key={{op://..}} ``` |
| `--user` | `LOCKET_FILE_OWNER` |  | Owner of the file/dir<br><br>Defaults to the running user/group. The running user must have write permissions on the directory to change the owner. |
| `--mode` | `LOCKET_INJECT_MODE` | `one-shot` | Mode of operation <br><br> **Choices:**<br>- `one-shot`: **Default** Materialize all secrets once and exit<br>- `watch`: **Docker Default** Watch for changes on templates and reinject<br>- `park`: Inject once and then park to keep the process alive |
| `--out` | `DEFAULT_SECRET_DIR` | `/run/secrets/locket` | Directory where secret values (literals) are materialized |
| `--inject-failure-policy` | `INJECT_POLICY` | `passthrough` | Policy for handling injection failures <br><br> **Choices:**<br>- `error`: Failures are treated as errors and will abort the process<br>- `passthrough`: On failure, copy the unmodified secret to destination<br>- `ignore`: On failure, ignore the secret and log a warning |
| `--max-file-size` | `MAX_FILE_SIZE` | `10M` | Maximum allowable size for a template file. Files larger than this will be rejected.<br><br>Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes). |
| `--file-mode` | `LOCKET_FILE_MODE` | `0600` | File permission mode |
| `--dir-mode` | `LOCKET_DIR_MODE` | `0700` | Directory permission mode |
| `--debounce` | `WATCH_DEBOUNCE` | `500ms` | Debounce duration for filesystem events in watch mode.<br><br>Events occurring within this duration will be coalesced into a single update so as to not overwhelm the secrets manager with rapid successive updates from filesystem noise.<br><br>Handles human-readable strings like "100ms", "2s", etc. Unitless numbers are interpreted as milliseconds. |
| `--log-format` | `LOCKET_LOG_FORMAT` | `text` | Log format <br><br> **Choices:**<br>- `text`: Plain text log format<br>- `json`: JSON log format<br>- `compose`: Special format for Docker Compose Provider specification |
| `--log-level` | `LOCKET_LOG_LEVEL` | `info` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |
### Provider Configuration

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
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
# Mode of operation
mode = "one-shot"

# Status file path used for healthchecks
# status-file = ...

# Mapping of source paths to destination paths
# 
# TOML syntax supports list of strings or map form:
# List form:
# map = ["/templates:/run/secrets/app", "/config:/run/secrets/config"]
# 
# Map form:
# [map]
# source = "/templates"
# destination = "/run/secrets/app"
# [map]
# source = "/config"
# destination = "/run/secrets/config"
# 
map = []

# Additional secret values specified as LABEL=SECRET_TEMPLATE
# 
# TOML syntax supports list of strings or map form:
# List form:
# secrets = ["db_password={{..}}", "api_key={{..}}"]
# 
# Map form:
# [secrets]
# db_password = "{{..}}"
# api_key = "{{..}}"
# 
secrets = []

# Directory where secret values (literals) are materialized
out = "/run/secrets/locket"

# Policy for handling injection failures
inject-failure-policy = "passthrough"

# Maximum allowable size for a template file. Files larger than this will be rejected
max-file-size = "10M"

# File permission mode
file-mode = "0600"

# Directory permission mode
dir-mode = "0700"

# Owner of the file/dir
# user = ...

# Debounce duration for filesystem events in watch mode
debounce = "500ms"

# Log format
log-format = "text"

# Log level
log-level = "info"

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
