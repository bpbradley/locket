pub struct InjectConfig {
    mode: InjectMode,
    status_file: Option<StatusFile>,
    manager: SecretFileManagerConfig,
    debounce: DebounceDuration,
    logger: Logger,
    provider: ProviderConfig,
}
