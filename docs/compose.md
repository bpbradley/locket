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
| `--provider` | `SECRETS_PROVIDER` |  | Secrets provider backend to use <br><br> **Choices:**<br>- `op`: 1Password Service Account<br>- `op-connect`: 1Password Connect Provider<br>- `bws`: Bitwarden Secrets Provider<br>- `infisical`: Infisical Secrets Provider<br>- `bao`: OpenBao / HashiCorp Vault Provider |
| `--env-file` | `LOCKET_ENV_FILE` |  | Files containing environment variables which may contain secret references |
| `--env` | `LOCKET_ENV` |  | Environment variable overrides which may contain secret references |
| `--raw` | `LOCKET_RAW` | `false` | Inject variables with their exact names, without the provider service name prefix<br><br>By default Docker Compose prefixes injected variables with the provider service name, e.g. `SECRET` becomes `LOCKET_SECRET` for a service named `locket`. Enable this to inject variables as-is. Requires Docker Compose v5.2.0 or later, and avoiding name collisions between provider services becomes your responsibility <br><br> **Choices:**<br>- `true`<br>- `false` |
| `<service>` |  |  | Service name from Docker Compose |
### 1Password (op)

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--op-token` | `OP_SERVICE_ACCOUNT_TOKEN` |  | 1Password Service Account Token<br><br>Either provide the token directly or via a file with `file:` prefix |
| `--op-bridge` | `LOCKET_OP_BRIDGE` |  | Optional: Path to the locket-op-bridge binary<br><br>Overrides automatic discovery, which prefers a bridge embedded in this binary and otherwise expects `locket-op-bridge` next to the locket executable. PATH is never searched. |
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
### OpenBao / Vault Provider

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--bao-url` | `BAO_URL` |  | OpenBao / Vault server URL |
| `--bao-namespace` | `BAO_NAMESPACE` |  | OpenBao / Vault namespace (Enterprise/OpenBao Namespaces feature) |
| `--bao-auth-mount` | `BAO_AUTH_MOUNT` |  | Auth mount path where the AppRole auth method is enabled |
| `--bao-role-id` | `BAO_ROLE_ID` |  | AppRole Role ID |
| `--bao-secret-id` | `BAO_SECRET_ID` |  | AppRole Secret ID<br><br>Either provide the value directly or via a file with `file:` prefix |
| `--bao-max-concurrent` | `BAO_MAX_CONCURRENT` |  | Maximum allowed concurrent requests to the OpenBao/Vault API |
| `--log-level` | `LOCKET_LOG_LEVEL` | `debug` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |

---

## `locket compose down`

Handler for Docker Compose `down`, but no-op because secrets are not persisted

### Options

| Command | Env | Default | Description |
| :--- | :--- | :--- | :--- |
| `--log-level` | `LOCKET_LOG_LEVEL` | `debug` | Log level <br><br> **Choices:**<br>- `trace`<br>- `debug`<br>- `info`<br>- `warn`<br>- `error` |
| `<service>` |  |  | Service name from Docker Compose |

---

## `locket compose metadata`

Handler for Docker Compose `metadata` command so that docker can query plugin capabilities

_No options._

