//! `t-sirji-fs.toml` — a device's own config, in a device's own home.
//!
//! A device is not a lightweight thing borrowing its parent's identity. It has its
//! own directory, its own key, and it knows where its parent is. That is what
//! makes "a device may be on another machine" true rather than aspirational:
//! nothing here assumes the parent's directory is readable.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "t-sirji-fs.toml";

/// Overrides where this device keeps its home.
pub const HOME_ENV: &str = "TSF_HOME";
/// Where the home lives when `TSF_HOME` is unset, under `$HOME`.
pub const HOME_DEFAULT: &str = ".t-sirji-fs";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The name we claim at our parent. Peers address us as `name@<parent>`.
    pub name: String,

    /// Our own key. We listen on it, because a peer that has resolved our name
    /// dials it directly — the parent is a doorman, never a proxy.
    pub key: String,

    /// Our parent's handshake keys: where we dial to register.
    pub parent: Vec<String>,

    /// Our parent's domains, if it has any, so its addresses can be refetched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_dns: Vec<String>,

    /// Socket hints for the parent, from the invite. Point-in-time and may be
    /// stale; the keys above are what endure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_hints: Vec<String>,

    /// The directory we serve. Everything under it is readable by any peer the
    /// parent grants a ticket to; nothing outside it is reachable at all.
    pub root: PathBuf,
}

impl Config {
    pub fn home() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os(HOME_ENV) {
            return Ok(PathBuf::from(dir));
        }
        let home = std::env::var_os("HOME")
            .context("neither TSF_HOME nor HOME is set; cannot find the device home")?;
        Ok(PathBuf::from(home).join(HOME_DEFAULT))
    }

    pub fn path_in(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    pub fn load(home: &Path) -> Result<Self> {
        let path = Self::path_in(home);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let path = Self::path_in(home);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Resolve a peer-supplied relative path inside `root`, refusing anything that
    /// would escape it.
    ///
    /// The check is on the **canonical** path, not the string: `..`, a symlink
    /// pointing out of the tree, and an absolute path all have to fail, and only
    /// canonicalising catches the symlink.
    pub fn resolve(&self, relative: &str) -> Result<PathBuf> {
        let root = self
            .root
            .canonicalize()
            .with_context(|| format!("the served root {} does not exist", self.root.display()))?;

        let candidate = root.join(relative.trim_start_matches('/'));
        let real = candidate
            .canonicalize()
            .with_context(|| format!("no such path: {relative}"))?;

        if !real.starts_with(&root) {
            anyhow::bail!("{relative} is outside the served directory");
        }
        Ok(real)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, Config) {
        let home = std::env::temp_dir().join(format!("tsf-test-{}", std::process::id()));
        let root = home.join("served");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("hello.txt"), b"hello").unwrap();
        std::fs::write(root.join("sub/nested.txt"), b"nested").unwrap();
        std::fs::write(home.join("secret.txt"), b"not yours").unwrap();
        let config = Config {
            name: "fs".into(),
            key: "k".into(),
            parent: vec![],
            parent_dns: vec![],
            parent_hints: vec![],
            root,
        };
        (home, config)
    }

    #[test]
    fn resolves_inside_the_root() {
        let (_home, config) = fixture();
        assert!(config.resolve("hello.txt").unwrap().ends_with("hello.txt"));
        assert!(config.resolve("sub/nested.txt").unwrap().ends_with("nested.txt"));
        assert!(config.resolve("/hello.txt").unwrap().ends_with("hello.txt"));
    }

    #[test]
    fn refuses_to_escape_with_dotdot() {
        let (_home, config) = fixture();
        let err = config.resolve("../secret.txt").unwrap_err().to_string();
        assert!(err.contains("outside") || err.contains("no such path"), "{err}");
    }

    #[test]
    fn refuses_a_symlink_out_of_the_tree() {
        let (home, config) = fixture();
        let link = config.root.join("escape");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.join("secret.txt"), &link).unwrap();

        // The string looks harmless; only canonicalising reveals where it goes.
        let err = config.resolve("escape").unwrap_err().to_string();
        assert!(err.contains("outside"), "{err}");
    }

    #[test]
    fn refuses_a_missing_path() {
        let (_home, config) = fixture();
        assert!(config.resolve("nope.txt").is_err());
    }
}
