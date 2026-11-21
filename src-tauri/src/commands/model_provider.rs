use std::{fs, io::Write, net::IpAddr, path::PathBuf};

use super::provider_bridge::ProviderBridge;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::process::Command;

pub(crate) async fn check_cli_support(path: &str) -> Result<(), String> {
    // Use a baseline verified to honor host-controlled routing and credential scrubbing.
    const MIN_VERSION: [u32; 3] = [2, 1, 329];
    let mut command = Command::from(crate::process::background_command(path));
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        command.arg("--version").kill_on_drop(true).output(),
    )
    .await;
    if let Ok(Ok(output)) = output {
        let version = String::from_utf8_lossy(&output.stdout);
        if output.status.success()
            && version
                .split_whitespace()
                .next()
                .and_then(parse_version)
                .is_some_and(|v| v >= MIN_VERSION)
        {
            return Ok(());
        }
    }
    Err("Custom providers require Claude Code 2.1.329 or later.".into())
}

fn parse_version(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.split('.');
    let numbers = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(numbers)
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
struct Provider {
    enabled: bool,
    base_url: String,
    model: String,
    auth: Auth,
    #[serde(default)]
    protocol: Protocol,
}

#[derive(Default, Deserialize, Serialize)]
struct SavedProvider {
    #[serde(flatten)]
    provider: Provider,
    credential: String,
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

fn load(app: &AppHandle) -> Result<SavedProvider, String> {
    match fs::read(config_path(app)?) {
        Ok(content) => serde_json::from_slice(&content)
            .map_err(|_| "Invalid model provider configuration".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SavedProvider::default()),
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

#[tauri::command]
pub fn get_model_provider(app: AppHandle) -> Result<ProviderStatus, String> {
    load(&app).map(Into::into)
}

#[tauri::command]
pub fn save_model_provider(
    app: AppHandle,
    update: ProviderUpdate,
) -> Result<ProviderStatus, String> {
    let previous = load(&app)?;
    let mut provider = update.provider;
    provider.base_url = provider.base_url.trim().to_string();
    provider.model = provider.model.trim().to_string();

    if !provider.base_url.is_empty() || provider.enabled {
        provider.base_url = normalized_url(&provider.base_url)?;
        super::provider_bridge::validate_endpoint(provider.protocol, &provider.base_url)?;
    }
    if (provider.enabled && provider.model.is_empty())
        || provider.model.starts_with('-')
        || provider
            .model
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("Enter a valid model ID.".into());
    }
    let credential = match update.credential {
        Some(value) if value.is_empty() => String::new(),
        Some(value) => {
            if value.bytes().any(|b| !(0x21..=0x7e).contains(&b)) {
                return Err("Enter a valid API credential.".into());
            }
            protect_credential(&value)?
        }
        None if provider.base_url == previous.provider.base_url
            && provider.auth == previous.provider.auth
            && provider.protocol == previous.provider.protocol =>
        {
            previous.credential
        }
        None => String::new(),
    };
    if provider.enabled && credential.is_empty() {
        return Err("Enter an API credential for this endpoint and authentication method.".into());
    }

    let saved = SavedProvider {
        provider,
        credential,
    };
    let path = config_path(&app)?;
    let dir = path
        .parent()
        .ok_or("Missing model provider configuration directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(temp.as_file_mut(), &saved).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.error.to_string())?;
    Ok(saved.into())
}

pub(crate) struct ActiveProvider {
    base_url: String,
    pub model: String,
    auth: Auth,
    protocol: Protocol,
    credential: String,
}

impl ActiveProvider {
    pub(crate) fn load(app: &AppHandle) -> Result<Option<Self>, String> {
        let SavedProvider {
            provider,
            credential,
        } = load(app)?;
        if !provider.enabled {
            return Ok(None);
        }
        if credential.is_empty() || provider.model.is_empty() {
            return Err("Incomplete model provider configuration".into());
        }
        Ok(Some(Self {
            base_url: normalized_url(&provider.base_url)?,
            model: provider.model,
            auth: provider.auth,
            protocol: provider.protocol,
            credential: unprotect_credential(&credential)?,
        }))
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
