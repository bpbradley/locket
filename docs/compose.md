[Return to Index](./CONFIGURATION.md)

> [!TIP]
> All configuration options can be set via command line arguments OR environment variables. CLI arguments take precedence.

## `locket compose`

Docker Compose provider API

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--project-name` | `COMPOSE_PROJECT_NAME` |  | Compose Project Name |

---

## `locket compose up`

Injects secrets into a Docker Compose service environment with `docker compose up`

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br><br> **Choices:**<br>- `op`: 1Password Service Account<br>- `op-connect`: 1Password Connect Provider<br>- `bws`: Bitwarden Secrets Provider<br>- `infisical`: Infisical Secrets Provider |
| `--env-file` | `LOCKET_ENV_FILE` |  | Files containing environment variables which may contain secret references |
| `--env` | `LOCKET_ENV` |  | Environment variable overrides which may contain secret references |
| `<service>` |  |  | Service name from Docker Compose |
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
| `--log-level` | `LOCKET_LOG_LEVEL` | `debug` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |

---

## `locket compose down`

Handler for Docker Compose `down`, but no-op because secrets are not persisted

_No options._


---

## `locket compose metadata`

Handler for Docker Compose `metadata` command so that docker can query plugin capabilities

_No options._

