//! `config.toml` — how this instance runs.
//!
//! Deliberately a separate file from `network.toml`. That one holds cryptographic
//! facts — whose key is whose — and is the same wherever you carry it. This holds
//! *operational* choices, which are properties of where a node happens to be
//! running: which relay is reachable from here, which CA this network makes you
//! trust. Two files because two lifetimes, not because two formats.
//!
//! Every setting can be overridden by an environment variable, so a single run can
//! differ without editing anything.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "config.toml";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Relays to use instead of iroh's defaults. Empty means direct only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relay: Vec<String>,

    /// Presented to access-controlled relays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_token: Option<String>,

    /// Extra CA certificates to trust: a PEM file, or a directory of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_ca: Option<PathBuf>,
}

/// Process-wide, because these are properties of the machine rather than of any
/// one call. Set once by whoever owns the home directory — a daemon, an app — and
/// read wherever an endpoint is bound.
static ACTIVE: OnceLock<Settings> = OnceLock::new();

impl Settings {
    pub fn path_in(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    /// Read `config.toml` if it exists, then let the environment override it.
    ///
    /// A missing file is not an error: the defaults plus environment are a
    /// complete configuration, and requiring the file would make the common case
    /// ceremonious for nothing.
    pub fn load(home: &Path) -> Result<Self> {
        let path = Self::path_in(home);
        let mut settings = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
        } else {
            Self::default()
        };
        settings.apply_env();
        Ok(settings)
    }

    /// Environment wins over file, so one run can differ without an edit.
    pub fn apply_env(&mut self) {
        if let Some(value) = std::env::var_os(crate::endpoint::RELAY_ENV) {
            self.relay = value
                .to_string_lossy()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Ok(token) = std::env::var(crate::endpoint::RELAY_TOKEN_ENV)
            && !token.is_empty()
        {
            self.relay_token = Some(token);
        }
        if let Some(path) = std::env::var_os(crate::endpoint::EXTRA_CA_ENV) {
            self.extra_ca = Some(PathBuf::from(path));
        }
    }

    /// Was `SIRJI_RELAY` set at all? An empty value means "no relays", which is
    /// different from "not configured".
    pub fn relays_configured(&self) -> bool {
        !self.relay.is_empty() || std::env::var_os(crate::endpoint::RELAY_ENV).is_some()
    }

    /// Make these the settings every endpoint binds with. Later calls are ignored.
    pub fn activate(self) {
        let _ = ACTIVE.set(self);
    }

    /// What endpoints should bind with. Falls back to environment alone when
    /// nothing has been activated — which is what a one-shot command wants.
    pub fn active() -> Self {
        ACTIVE.get().cloned().unwrap_or_else(|| {
            let mut settings = Self::default();
            settings.apply_env();
            settings
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_a_valid_default() {
        let home = std::env::temp_dir().join("sirji-settings-absent");
        let settings = Settings::load(&home).unwrap();
        assert!(settings.relay.is_empty());
        assert!(settings.extra_ca.is_none());
    }

    #[test]
    fn parses_what_it_writes() {
        let settings = Settings {
            relay: vec!["https://relay.example.com".into()],
            relay_token: Some("secret".into()),
            extra_ca: Some(PathBuf::from("/etc/sirji/relay.crt")),
        };
        let back: Settings = toml::from_str(&toml::to_string_pretty(&settings).unwrap()).unwrap();
        assert_eq!(back.relay, settings.relay);
        assert_eq!(back.relay_token, settings.relay_token);
        assert_eq!(back.extra_ca, settings.extra_ca);
    }

    #[test]
    fn an_empty_config_is_valid() {
        let settings: Settings = toml::from_str("").unwrap();
        assert!(settings.relay.is_empty());
    }
}
