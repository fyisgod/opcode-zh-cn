pub mod catalog;

use std::{collections::HashSet, fs, io::Write, net::IpAddr, path::PathBuf, sync::Mutex};

use super::provider_bridge::ProviderBridge;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::process::Command;

pub(crate) async fn check_cli_support(path: &str) -> Result<(), String> {
    // Use a baseline verified to honor host-controlled routing and credential scrubbing.
    if crate::claude_binary::cli_at_least(path, [2, 1, 329]).await {
        Ok(())
    } else {
        Err("Custom providers require Claude Code 2.1.329 or later.".into())
    }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Auth {
    #[default]
    Bearer,
    ApiKey,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Protocol {
    #[default]
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    Gemini,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConnection {
    enabled: bool,
    base_url: String,
    auth: Auth,
    #[serde(default)]
    protocol: Protocol,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reasoning_efforts: Vec<ReasoningEffort>,
}

impl ProviderModel {
    fn from_id(id: String) -> Self {
        Self {
            display_name: id.clone(),
            id,
            context_length: None,
            reasoning_efforts: Vec::new(),
        }
    }

    fn valid_id(id: &str) -> bool {
        !id.is_empty()
            && !id.starts_with('-')
            && !id.chars().any(|c| c.is_whitespace() || c.is_control())
    }

    fn normalize(&mut self) {
        self.id = self.id.trim().to_owned();
        self.display_name = self.display_name.trim().to_owned();
        if self.display_name.is_empty() {
            self.display_name.clone_from(&self.id);
        }
        let mut efforts = Vec::new();
        self.reasoning_efforts.retain(|effort| {
            if efforts.contains(effort) {
                return false;
            }
            efforts.push(*effort);
            true
        });
    }
}

fn deserialize_models<'de, D>(deserializer: D) -> Result<Vec<ProviderModel>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ModelEntry {
        Id(String),
        Model(ProviderModel),
    }

    Vec::<ModelEntry>::deserialize(deserializer).map(|entries| {
        entries
            .into_iter()
            .map(|entry| match entry {
                ModelEntry::Id(id) => ProviderModel::from_id(id),
                ModelEntry::Model(model) => model,
            })
            .collect()
    })
}

#[derive(Default, Deserialize, Serialize)]
struct Provider {
    id: String,
    name: String,
    #[serde(deserialize_with = "deserialize_models")]
    models: Vec<ProviderModel>,
    #[serde(flatten)]
    connection: ProviderConnection,
}

#[derive(Default, Deserialize, Serialize)]
struct SavedProvider {
    #[serde(flatten)]
    provider: Provider,
    credential: String,
}

#[derive(Default, Deserialize, Serialize)]
struct Registry {
    providers: Vec<SavedProvider>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ProviderFile {
    Registry(Registry),
    Legacy {
        #[serde(flatten)]
        connection: ProviderConnection,
        model: String,
        credential: String,
    },
}

const LEGACY_PROVIDER_ID: &str = "legacy-provider";
static REGISTRY_WRITE: Mutex<()> = Mutex::new(());

impl ProviderFile {
    fn into_registry(self) -> Registry {
        match self {
            Self::Registry(registry) => registry,
            Self::Legacy {
                connection,
                model,
                credential,
            } => {
                let name = Url::parse(&connection.base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "Custom provider".into());
                Registry {
                    providers: vec![SavedProvider {
                        provider: Provider {
                            id: LEGACY_PROVIDER_ID.into(),
                            name,
                            models: if model.is_empty() {
                                Vec::new()
                            } else {
                                vec![ProviderModel::from_id(model)]
                            },
                            connection,
                        },
                        credential,
                    }],
                }
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    #[serde(flatten)]
    provider: Provider,
    has_credential: bool,
}

impl From<SavedProvider> for ProviderStatus {
    fn from(saved: SavedProvider) -> Self {
        Self {
            provider: saved.provider,
            has_credential: !saved.credential.is_empty(),
        }
    }
}

#[derive(Deserialize)]
pub struct ProviderUpdate {
    #[serde(flatten)]
    provider: Provider,
    credential: Option<String>,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("model-provider.json"))
}

fn load(path: &std::path::Path) -> Result<Registry, String> {
    match fs::read(path) {
        Ok(content) => serde_json::from_slice::<ProviderFile>(&content)
            .map(ProviderFile::into_registry)
            .map_err(|_| "Invalid model provider configuration".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(format!("Could not read model provider configuration: {e}")),
    }
}

fn normalized_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "Enter a valid base URL.".to_string())?;
    let is_loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && is_loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_end_matches('/').ends_with("/v1/messages")
    {
        return Err("Use an HTTPS base URL, or HTTP on localhost, without credentials, query, fragment or /v1/messages.".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn validate_credential(value: &str) -> Result<(), String> {
    if value.is_empty() || value.bytes().any(|b| !(0x21..=0x7e).contains(&b)) {
        return Err("Enter a valid API credential.".into());
    }
    Ok(())
}

fn persist(path: &std::path::Path, registry: &Registry) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or("Missing model provider configuration directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(temp.as_file_mut(), registry).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

impl Registry {
    fn status(self) -> Vec<ProviderStatus> {
        self.providers.into_iter().map(Into::into).collect()
    }

    fn upsert(&mut self, update: ProviderUpdate) -> Result<String, String> {
        let mut provider = update.provider;
        let index = if provider.id.is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
            None
        } else {
            Some(
                self.providers
                    .iter()
                    .position(|saved| saved.provider.id == provider.id)
                    .ok_or("This model provider no longer exists.")?,
            )
        };

        provider.name = provider.name.trim().to_string();
        if provider.name.is_empty() || provider.name.chars().any(char::is_control) {
            return Err("Enter a provider name.".into());
        }
        let connection = &mut provider.connection;
        connection.base_url = connection.base_url.trim().to_string();
        if !connection.base_url.is_empty() || connection.enabled {
            connection.base_url = normalized_url(&connection.base_url)?;
            super::provider_bridge::validate_endpoint(connection.protocol, &connection.base_url)?;
        }
        let mut seen = HashSet::new();
        provider.models.retain_mut(|model| {
            model.normalize();
            !model.id.is_empty() && seen.insert(model.id.clone())
        });
        if (connection.enabled && provider.models.is_empty())
            || provider
                .models
                .iter()
                .any(|model| !ProviderModel::valid_id(&model.id))
        {
            return Err("Enter valid model IDs.".into());
        }
        if provider
            .models
            .iter()
            .any(|model| model.display_name.chars().any(char::is_control))
        {
            return Err("Enter a valid model display name.".into());
        }
        if provider
            .models
            .iter()
            .any(|model| model.context_length == Some(0))
        {
            return Err("Context length must be a positive whole number.".into());
        }
        let credential = match update.credential {
            Some(value) if value.is_empty() => String::new(),
            Some(value) => {
                validate_credential(&value)?;
                protect_credential(&value)?
            }
            None => index
                .and_then(|i| {
                    let previous = &self.providers[i];
                    let saved = &previous.provider.connection;
                    (connection.base_url == saved.base_url
                        && connection.auth == saved.auth
                        && connection.protocol == saved.protocol)
                        .then(|| previous.credential.clone())
                })
                .unwrap_or_default(),
        };
        if connection.enabled && credential.is_empty() {
            return Err(
                "Enter an API credential for this endpoint and authentication method.".into(),
            );
        }

        let id = provider.id.clone();
        let saved = SavedProvider {
            provider,
            credential,
        };
        if let Some(index) = index {
            self.providers[index] = saved;
        } else {
            self.providers.push(saved);
        }
        Ok(id)
    }
}

#[derive(Serialize)]
pub struct ProviderSaveResult {
    id: String,
    providers: Vec<ProviderStatus>,
}

#[tauri::command]
pub fn get_model_providers(app: AppHandle) -> Result<Vec<ProviderStatus>, String> {
    load(&config_path(&app)?).map(Registry::status)
}

#[tauri::command]
pub fn save_model_provider(
    app: AppHandle,
    update: ProviderUpdate,
) -> Result<ProviderSaveResult, String> {
    let _write = REGISTRY_WRITE
        .lock()
        .map_err(|_| "Could not save model provider settings.")?;
    let path = config_path(&app)?;
    let mut registry = load(&path)?;
    let id = registry.upsert(update)?;
    persist(&path, &registry)?;
    Ok(ProviderSaveResult {
        id,
        providers: registry.status(),
    })
}

#[tauri::command]
pub fn remove_model_provider(app: AppHandle, id: String) -> Result<Vec<ProviderStatus>, String> {
    let _write = REGISTRY_WRITE
        .lock()
        .map_err(|_| "Could not save model provider settings.")?;
    let path = config_path(&app)?;
    let mut registry = load(&path)?;
    registry.providers.retain(|saved| saved.provider.id != id);
    persist(&path, &registry)?;
    Ok(registry.status())
}

pub(crate) struct ActiveProvider {
    base_url: String,
    pub model: String,
    auth: Auth,
    protocol: Protocol,
    credential: String,
}

impl ActiveProvider {
    pub(crate) fn load(app: &AppHandle, id: &str, model: &str) -> Result<Self, String> {
        Self::resolve(load(&config_path(app)?)?, id, model)
    }

    fn resolve(registry: Registry, id: &str, model: &str) -> Result<Self, String> {
        let saved = registry
            .providers
            .into_iter()
            .find(|saved| {
                saved.provider.id == id
                    && saved.provider.connection.enabled
                    && !saved.credential.is_empty()
                    && saved
                        .provider
                        .models
                        .iter()
                        .any(|candidate| candidate.id == model)
            })
            .ok_or("The selected model is no longer available.")?;
        let connection = saved.provider.connection;
        Ok(Self {
            base_url: normalized_url(&connection.base_url)?,
            model: model.to_owned(),
            auth: connection.auth,
            protocol: connection.protocol,
            credential: unprotect_credential(&saved.credential)?,
        })
    }

    pub(super) async fn configure(
        &self,
        cmd: &mut Command,
    ) -> Result<Option<ProviderBridge>, String> {
        if self.protocol == Protocol::Anthropic {
            self.apply(cmd, &self.base_url, self.auth, &self.credential);
            return Ok(None);
        }

        let bridge = ProviderBridge::start(
            self.protocol,
            &self.base_url,
            &self.model,
            self.auth,
            &self.credential,
        )
        .await?;
        self.apply(cmd, &bridge.base_url, Auth::Bearer, &bridge.token);
        let inherited = std::env::var("NO_PROXY")
            .or_else(|_| std::env::var("no_proxy"))
            .unwrap_or_default();
        let no_proxy = if inherited.trim().is_empty() {
            "127.0.0.1".into()
        } else {
            format!("127.0.0.1,{inherited}")
        };
        #[cfg(not(windows))]
        cmd.env("no_proxy", &no_proxy);
        cmd.env("NO_PROXY", no_proxy)
            .env("DISABLE_PROMPT_CACHING", "1")
            .env("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1")
            .env("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1")
            .env("ENABLE_TOOL_SEARCH", "false");
        Ok(Some(bridge))
    }

    fn apply(&self, cmd: &mut Command, base_url: &str, auth: Auth, credential: &str) {
        // Keep user and project settings from overriding the host's provider routing.
        for var in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_CUSTOM_HEADERS",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
            "CLAUDE_CODE_WEBAUTH_TOKEN",
            "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_ANTHROPIC_AWS",
            "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
            "CLAUDE_CODE_USE_MANTLE",
            "ANTHROPIC_FALLBACK_MODEL",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")
            .env("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB", "1")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("ANTHROPIC_BASE_URL", base_url)
            .env("ANTHROPIC_MODEL", &self.model)
            .env("ANTHROPIC_DEFAULT_SONNET_MODEL", &self.model)
            .env("ANTHROPIC_DEFAULT_OPUS_MODEL", &self.model)
            .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", &self.model)
            .env("ANTHROPIC_SMALL_FAST_MODEL", &self.model)
            .env(
                match auth {
                    Auth::Bearer => "ANTHROPIC_AUTH_TOKEN",
                    Auth::ApiKey => "ANTHROPIC_API_KEY",
                },
                credential,
            );
    }
}

#[cfg(windows)]
fn dpapi(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: input
            .len()
            .try_into()
            .map_err(|_| "Credential is too large")?,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI binds the encrypted blob to the current Windows user.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return Err("Could not access the Windows-protected API credential".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}

#[cfg(windows)]
fn protect_credential(value: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    Ok(STANDARD.encode(dpapi(value.as_bytes(), true)?))
}

#[cfg(windows)]
fn unprotect_credential(value: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| "Invalid saved API credential")?;
    String::from_utf8(dpapi(&bytes, false)?).map_err(|_| "Invalid saved API credential".into())
}

// On Unix, NamedTempFile creates the settings file with owner-only permissions.
#[cfg(not(windows))]
fn protect_credential(value: &str) -> Result<String, String> {
    Ok(value.to_string())
}

#[cfg(not(windows))]
fn unprotect_credential(value: &str) -> Result<String, String> {
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_safe_urls_and_rejects_unsafe_ones() {
        assert_eq!(
            normalized_url("https://EXAMPLE.com/gateway/").unwrap(),
            "https://example.com/gateway"
        );
        assert!(normalized_url("http://[::1]:9876").is_ok());
        assert!(normalized_url("http://127.0.0.1:9876").is_ok());
        for invalid in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com#secret",
            "https://example.com?a=b",
            "https://example.com/v1/messages/",
        ] {
            assert!(normalized_url(invalid).is_err());
        }
    }

    #[test]
    fn provider_status_does_not_expose_credentials() {
        let value = serde_json::to_value(ProviderStatus::from(SavedProvider {
            provider: Provider::default(),
            credential: "test-secret".into(),
        }))
        .unwrap();
        assert_eq!(value["hasCredential"], true);
        assert!(value.get("credential").is_none());
    }

    fn update(id: &str, url: &str, models: &[&str], credential: Option<&str>) -> ProviderUpdate {
        ProviderUpdate {
            provider: Provider {
                id: id.into(),
                name: "Test provider".into(),
                models: models
                    .iter()
                    .map(|model| ProviderModel::from_id(model.to_string()))
                    .collect(),
                connection: ProviderConnection {
                    enabled: true,
                    base_url: url.into(),
                    auth: Auth::Bearer,
                    protocol: Protocol::Anthropic,
                },
            },
            credential: credential.map(str::to_owned),
        }
    }

    #[test]
    fn migrates_legacy_provider_without_changing_its_credential() {
        let legacy: ProviderFile = serde_json::from_value(serde_json::json!({
            "enabled": true, "baseUrl": "https://legacy.example/v1", "model": "legacy-model",
            "auth": "apiKey", "credential": "opaque-encrypted-credential"
        }))
        .unwrap();
        let mut registry = legacy.into_registry();
        let saved = &registry.providers[0];
        assert_eq!(saved.provider.id, LEGACY_PROVIDER_ID);
        assert_eq!(saved.provider.name, "legacy.example");
        assert_eq!(
            saved.provider.models,
            [ProviderModel::from_id("legacy-model".into())]
        );
        assert!(saved.provider.connection.enabled);
        assert_eq!(saved.provider.connection.protocol, Protocol::Anthropic);
        assert_eq!(saved.credential, "opaque-encrypted-credential");

        registry
            .upsert(update(
                "",
                "https://second.example",
                &["second-model"],
                Some("second-key"),
            ))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-provider.json");
        persist(&path, &registry).unwrap();
        let stored = load(&path).unwrap();
        assert_eq!(stored.providers.len(), 2);
        assert_eq!(
            stored.providers[0].credential,
            "opaque-encrypted-credential"
        );
    }

    #[test]
    fn round_trips_metadata_alongside_existing_model_ids() {
        let update = serde_json::from_value(serde_json::json!({
            "id": "", "name": "Test", "enabled": true, "baseUrl": "https://test.example",
            "auth": "bearer", "protocol": "anthropic", "credential": "dummy-key",
            "models": ["old", {"id": "new", "displayName": "New model", "contextLength": 200000,
                "reasoningEfforts": ["low", "high", "high"]}]
        }))
        .unwrap();
        let mut registry = Registry::default();
        let id = registry.upsert(update).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-provider.json");
        persist(&path, &registry).unwrap();
        let stored = load(&path).unwrap();
        let models = &stored.providers[0].provider.models;
        assert_eq!(models[0], ProviderModel::from_id("old".into()));
        assert_eq!(models[1].display_name, "New model");
        assert_eq!(models[1].context_length, Some(200_000));
        assert_eq!(
            models[1].reasoning_efforts,
            [ReasoningEffort::Low, ReasoningEffort::High]
        );
        assert_eq!(
            ActiveProvider::resolve(stored, &id, "new").unwrap().model,
            "new"
        );
    }

    #[test]
    fn uses_the_selected_provider_and_model_even_with_duplicate_model_ids() {
        let mut registry = Registry::default();
        let a = registry
            .upsert(update(
                "",
                "https://a.example",
                &["same", " alternate ", "same"],
                Some("key-a"),
            ))
            .unwrap();
        let b = registry
            .upsert(update("", "https://b.example", &["same"], Some("key-b")))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-provider.json");
        persist(&path, &registry).unwrap();
        let resolve = |id, model| ActiveProvider::resolve(load(&path).unwrap(), id, model);
        for (id, model, endpoint, key) in [
            (&a, "same", "https://a.example", "key-a"),
            (&a, "alternate", "https://a.example", "key-a"),
            (&b, "same", "https://b.example", "key-b"),
        ] {
            let active = resolve(id, model).unwrap();
            assert_eq!(active.base_url, endpoint);
            assert_eq!(active.credential, key);
            assert_eq!(active.model, model);
        }
        assert_eq!(
            registry.providers[0]
                .provider
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["same", "alternate"]
        );
        assert!(resolve(&b, "alternate").is_err());
        let mut disabled = update(&a, "https://a.example", &["same"], None);
        disabled.provider.connection.enabled = false;
        registry.upsert(disabled).unwrap();
        assert!(ActiveProvider::resolve(registry, &a, "same").is_err());
    }

    #[test]
    fn keeps_keys_when_editing_models_but_not_when_changing_endpoints() {
        let mut registry = Registry::default();
        let id = registry
            .upsert(update(
                "",
                "https://a.example",
                &["first"],
                Some("first-key"),
            ))
            .unwrap();
        registry
            .upsert(update(&id, "https://a.example", &["second"], None))
            .unwrap();
        assert!(registry
            .upsert(update(&id, "https://b.example", &["second"], None))
            .is_err());
        let active = ActiveProvider::resolve(registry, &id, "second").unwrap();
        assert_eq!(active.base_url, "https://a.example");
        assert_eq!(active.credential, "first-key");
    }

    #[test]
    fn sets_only_the_selected_authentication_header() {
        for (auth, selected, other) in [
            (Auth::Bearer, "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"),
            (Auth::ApiKey, "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"),
        ] {
            let mut cmd = Command::new("claude");
            let provider = ActiveProvider {
                base_url: "https://example.com".into(),
                model: "test-model".into(),
                auth,
                protocol: Protocol::Anthropic,
                credential: "test-secret".into(),
            };
            provider.apply(
                &mut cmd,
                &provider.base_url,
                provider.auth,
                &provider.credential,
            );
            let env: std::collections::HashMap<_, _> = cmd.as_std().get_envs().collect();
            assert_eq!(env[std::ffi::OsStr::new(selected)].unwrap(), "test-secret");
            assert_eq!(env[std::ffi::OsStr::new(other)], None);
            assert_eq!(
                env[std::ffi::OsStr::new("ANTHROPIC_MODEL")].unwrap(),
                "test-model"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_encrypts_and_recovers_credentials() {
        let ciphertext = protect_credential("test-secret").unwrap();
        assert_ne!(ciphertext, "test-secret");
        assert_eq!(unprotect_credential(&ciphertext).unwrap(), "test-secret");
    }
}
