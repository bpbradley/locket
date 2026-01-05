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
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br> **Choices:** `op`, `op-connect`, `bws` |
| `--env-file` | `LOCKET_ENV_FILE` |  | Files containing environment variables which may contain secret references |
| `--env` | `LOCKET_ENV` |  | Environment variable overrides which may contain secret references |
| `<service>` |  |  | Service name from Docker Compose |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op.token` | `OP_SERVICE_ACCOUNT_TOKEN` |  | 1Password Service Account Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--op.config-dir` | `OP_CONFIG_DIR` |  | Optional: Path to 1Password config directory<br><br>Defaults to standard op config locations if not provided, e.g. $XDG_CONFIG_HOME/op |
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
| `--log-level` | `LOCKET_LOG_LEVEL` | `debug` | Log level <br> **Choices:** `trace`, `debug`, `info`, `warn`, `error` |

---

## `locket compose down`

Handler for Docker Compose `down`, but no-op because secrets are not persisted

_No options._


---

## `locket compose metadata`

Handler for Docker Compose `metadata` command so that docker can query plugin capabilities

_No options._

