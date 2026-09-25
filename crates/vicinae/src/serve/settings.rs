//! The settings view's writes (`GeneralSettingsModel`'s setters, and the
//! extension page's provider switch): one setting of `vicinae.json` at a
//! time, checked against `compass_core::settings_catalog`, then applied to
//! what the engine already holds where it holds it.

use std::sync::Arc;

use compass_core::Config;
use compass_core::settings_catalog::{self, Kind};
use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

fn bad_request(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::BadRequest, message))
}

fn internal(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::Internal, message))
}

/// [`Request::SetSetting`] and [`Request::SetProviderEnabled`].
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::SetSetting { key, value_json } => set_setting(state, key, &value_json).await,
        Request::SetProviderEnabled { provider, enabled } => {
            set_provider_enabled(state, provider, enabled).await
        }
        _ => bad_request("not a settings request"),
    }
}

/// Reads the file, changes it with `change` and writes it back. A file that
/// does not parse is left alone rather than replaced by one holding only
/// this change.
async fn rewrite(
    change: impl FnOnce(&mut Config) -> Result<(), String> + Send + 'static,
) -> Result<Config, Response> {
    let saved = tokio::task::spawn_blocking(move || -> Result<Config, String> {
        let mut config =
            Config::load().map_err(|err| format!("could not read the configuration: {err}"))?;
        change(&mut config)?;
        let path = compass_core::config::default_config_path()
            .map_err(|err| format!("could not save the configuration: {err}"))?;
        config
            .save_to(path)
            .map_err(|err| format!("could not save the configuration: {err}"))?;
        Ok(config)
    })
    .await;
    match saved {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(reason)) => Err(internal(reason)),
        Err(err) => Err(internal(format!("the settings task failed: {err}"))),
    }
}

async fn set_setting(state: &Arc<RwLock<EngineState>>, key: String, value_json: &str) -> Response {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(value_json) else {
        return bad_request("the setting's value is not JSON");
    };
    let Some(setting) = settings_catalog::find(&key) else {
        return bad_request(format!("{key:?} is not a setting"));
    };
    let value = match settings_catalog::validate(&setting, value) {
        Ok(value) => value,
        Err(reason) => return bad_request(reason),
    };
    // A theme is kept by the name the theme list knows it by, as Set Theme
    // keeps it; one that is not a theme is refused here rather than written.
    let value = match (&setting.kind, value) {
        (Kind::Theme, Some(serde_json::Value::String(name))) => {
            let _ = tokio::task::spawn_blocking(compass_ui::theme::load_default_user_themes).await;
            match compass_ui::theme::Theme::from_name(&name) {
                Some(theme) => Some(serde_json::Value::String(theme.name().to_owned())),
                None => return bad_request(format!("unknown theme {name:?}")),
            }
        }
        (_, value) => value,
    };
    let path = setting.key.clone();
    let config = match rewrite(move |config| config.set_path(&path, value)).await {
        Ok(config) => config,
        Err(response) => return response,
    };
    apply_live(state, &config, &setting.key).await;
    Response::Ack
}

/// Hands what the engine holds in memory the new value, where it holds it;
/// the rest is read where it is used, or when the launcher next starts.
async fn apply_live(state: &Arc<RwLock<EngineState>>, config: &Config, key: &str) {
    let expander = {
        let mut state = state.write().await;
        if key == "launcher.max_results" {
            state.max_results = config.launcher().max_results();
        }
        if key.starts_with("providers.clipboard.") {
            state
                .clipboard_control
                .apply(&crate::clipboard_service::Settings::from_preferences(
                    config.provider_preferences(crate::clipboard_service::PROVIDER_ID),
                ));
        }
        if key.contains(&format!(".{}.", crate::programs::ENTRYPOINT)) {
            state.run_program_default =
                crate::programs::default_action(config.entrypoint_preferences(
                    compass_core::commands::COMMANDS_PROVIDER_ID,
                    crate::programs::ENTRYPOINT,
                ));
        }
        state.expander.clone()
    };
    if key == "input_server.enabled"
        && let Some(expander) = expander
    {
        expander
            .server()
            .set_enabled(config.input_server().enabled());
    }
}

async fn set_provider_enabled(
    state: &Arc<RwLock<EngineState>>,
    provider: String,
    enabled: bool,
) -> Response {
    if provider.is_empty() || provider.contains(':') {
        return bad_request(format!("{provider:?} is not a provider id"));
    }
    let config = match rewrite(move |config| {
        config.set_provider_enabled(&provider, enabled);
        Ok(())
    })
    .await
    {
        Ok(config) => config,
        Err(response) => return response,
    };
    state
        .write()
        .await
        .index
        .apply_root_config(&config.root_config());
    Response::Ack
}
