/// Typed access to the environment Nebo launches an app's sidecar with.
#[derive(Debug, Clone)]
pub struct AppEnv {
    /// `NEBO_APP_DIR`: the app's code folder (replaced on every update).
    pub dir: String,
    /// `NEBO_APP_ID`: the app's id.
    pub id: String,
    /// `NEBO_APP_NAME`: the app's name.
    pub name: String,
    /// `NEBO_APP_VERSION`: the app's version.
    pub version: String,
    /// `NEBO_DATA_DIR`: the app's own data folder, kept across updates; also
    /// the sidecar's working directory.
    pub data_dir: String,
    /// `NEBO_API_URL`: Nebo's local API, `http://127.0.0.1:<port>`.
    pub api_url: String,
    /// `NEBO_APP_TOKEN`: this launch's secret. Send it as `Authorization:
    /// Bearer <token>` to `api_url`; Nebo sends it on every call to the
    /// sidecar, and the SDK refuses calls without it.
    pub token: String,
}

impl AppEnv {
    pub fn load() -> Self {
        let var = |name| std::env::var(name).unwrap_or_default();
        Self {
            dir: var("NEBO_APP_DIR"),
            id: var("NEBO_APP_ID"),
            name: var("NEBO_APP_NAME"),
            version: var("NEBO_APP_VERSION"),
            data_dir: var("NEBO_DATA_DIR"),
            api_url: var("NEBO_API_URL"),
            token: var("NEBO_APP_TOKEN"),
        }
    }
}
