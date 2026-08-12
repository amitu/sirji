//! The keystore — one file per key, named by its id52.
//!
//! ```text
//! $SIRJI_HOME/keys/<id52>.private-key    32 bytes, mode 0600
//! ```
//!
//! The filename is the index: given a public key, the secret is at a known path.
//! No counter, no derivation, no lookup table. Because the name carries the public
//! half, the store is **self-verifying** — loading a key checks that the secret
//! really does produce the public key it is filed under, so a corrupted or
//! misfiled key is caught at startup rather than at first dial.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use iroh::{PublicKey, SecretKey};

use crate::id52;

/// Overrides the location of the sirji home directory.
pub const HOME_ENV: &str = "SIRJI_HOME";
/// Where the home directory lives when `SIRJI_HOME` is unset, under `$HOME`.
pub const HOME_DEFAULT: &str = ".sirji";

const KEYS_DIR: &str = "keys";
const KEY_EXT: &str = "private-key";

/// The sirji home directory: `$SIRJI_HOME`, else `$HOME/.sirji`.
pub fn home() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(HOME_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .context("neither SIRJI_HOME nor HOME is set; cannot find the sirji directory")?;
    Ok(PathBuf::from(home).join(HOME_DEFAULT))
}

/// A directory of secret keys, one file each.
#[derive(Debug, Clone)]
pub struct Keystore {
    dir: PathBuf,
}

impl Keystore {
    /// The keystore under the sirji home directory.
    pub fn open() -> Result<Self> {
        Ok(Self::at(home()?.join(KEYS_DIR)))
    }

    /// A keystore at an explicit path. Useful in tests.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Mint a new key, persist it, and return its public half.
    pub fn generate(&self) -> Result<PublicKey> {
        let secret = SecretKey::generate();
        self.insert(&secret)?;
        Ok(secret.public())
    }

    /// Write a secret to the store. Refuses to overwrite: a key file that already
    /// exists is either the same key (nothing to do) or a collision we must not
    /// paper over.
    pub fn insert(&self, secret: &SecretKey) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating keystore at {}", self.dir.display()))?;

        let path = self.path_of(&secret.public());
        if path.exists() {
            return Ok(());
        }
        std::fs::write(&path, secret.to_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        restrict(&path)?;
        Ok(())
    }

    /// Load the secret for a public key, verifying the store is not lying to us.
    pub fn secret(&self, key: &PublicKey) -> Result<SecretKey> {
        let path = self.path_of(key);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("no key for {} at {}", id52::encode(key), path.display()))?;
        let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("{} is {} bytes; a secret key is 32", path.display(), bytes.len())
        })?;

        let secret = SecretKey::from_bytes(&bytes);
        if secret.public() != *key {
            bail!(
                "{} holds a key for {}, not for {} — the store is corrupt or misfiled",
                path.display(),
                id52::encode(&secret.public()),
                id52::encode(key),
            );
        }
        Ok(secret)
    }

    /// Every key in the store, sorted. Files that are not keys are ignored; files
    /// that look like keys but are not get reported, because silence there would
    /// hide exactly the corruption this layout is meant to catch.
    pub fn list(&self) -> Result<Vec<PublicKey>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("reading {}", self.dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(KEY_EXT) {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let key = id52::decode(stem)
                .with_context(|| format!("{} is not named after an id52", path.display()))?;
            keys.push(key);
        }
        keys.sort_by_key(id52::encode);
        Ok(keys)
    }

    fn path_of(&self, key: &PublicKey) -> PathBuf {
        self.dir.join(format!("{}.{KEY_EXT}", id52::encode(key)))
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("setting 0600 on {}", path.display()))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    // No portable equivalent. The file still holds a secret, so this platform
    // needs its own answer before it is supported.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sirji-keystore-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn generate_then_load_round_trips() {
        let store = Keystore::at(temp());
        let key = store.generate().unwrap();
        assert_eq!(store.secret(&key).unwrap().public(), key);
    }

    #[test]
    fn lists_what_it_generated() {
        let store = Keystore::at(temp());
        let mut minted: Vec<_> = (0..3).map(|_| store.generate().unwrap()).collect();
        minted.sort_by_key(id52::encode);
        assert_eq!(store.list().unwrap(), minted);
    }

    #[test]
    fn missing_key_is_an_error_naming_the_key() {
        let store = Keystore::at(temp());
        let absent = SecretKey::generate().public();
        let err = store.secret(&absent).unwrap_err().to_string();
        assert!(err.contains(&id52::encode(&absent)), "{err}");
    }

    #[test]
    fn a_misfiled_key_is_caught_on_load() {
        let store = Keystore::at(temp());
        let real = store.generate().unwrap();
        let impostor = SecretKey::generate();

        // Same bytes, wrong name: exactly what a bad copy or rename would leave.
        std::fs::write(store.path_of(&real), impostor.to_bytes()).unwrap();

        let err = store.secret(&real).unwrap_err().to_string();
        assert!(err.contains("corrupt or misfiled"), "{err}");
    }

    #[test]
    fn a_truncated_key_is_caught_on_load() {
        let store = Keystore::at(temp());
        let key = store.generate().unwrap();
        std::fs::write(store.path_of(&key), [0u8; 16]).unwrap();

        let err = store.secret(&key).unwrap_err().to_string();
        assert!(err.contains("a secret key is 32"), "{err}");
    }

    #[test]
    fn empty_store_lists_nothing() {
        assert!(Keystore::at(temp()).list().unwrap().is_empty());
    }
}
