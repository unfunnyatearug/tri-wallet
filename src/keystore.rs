use anyhow::{anyhow, bail, Context, Result};
use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const M_COST: u32 = 65536; // 64 MiB
const T_COST: u32 = 3;
const P_COST: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Keystore {
    pub version: u32,
    /// false means the file is encrypted with an empty passphrase, which
    /// protects nothing. Kept as a flag so the tool can warn on every use.
    pub protected: bool,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    let params = argon2::Params::new(M_COST, T_COST, P_COST, Some(32))
        .map_err(|e| anyhow!("argon2 params: {e}"))?;
    let a2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    a2.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(key)
}

impl Keystore {
    pub fn seal(mnemonic: &str, passphrase: &str) -> Result<Keystore> {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut salt);
        rand::thread_rng().fill_bytes(&mut nonce);
        let mut key = derive_key(passphrase, &salt)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), mnemonic.as_bytes())
            .map_err(|_| anyhow!("encryption failed"))?;
        key.zeroize();
        Ok(Keystore {
            version: 1,
            protected: !passphrase.is_empty(),
            salt: hex::encode(salt),
            nonce: hex::encode(nonce),
            ciphertext: hex::encode(ct),
        })
    }

    pub fn open(&self, passphrase: &str) -> Result<String> {
        let salt = hex::decode(&self.salt).context("bad salt")?;
        let nonce = hex::decode(&self.nonce).context("bad nonce")?;
        let ct = hex::decode(&self.ciphertext).context("bad ciphertext")?;
        let mut key = derive_key(passphrase, &salt)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let pt = cipher
            .decrypt(XNonce::from_slice(&nonce), ct.as_ref())
            .map_err(|_| anyhow!("wrong passphrase, or the wallet file is corrupt"))?;
        key.zeroize();
        Ok(String::from_utf8(pt)?)
    }
}

pub fn data_dir() -> Result<PathBuf> {
    // TRI_HOME allows keeping the wallet somewhere other than the home
    // directory, for example on removable storage.
    if let Ok(dir) = std::env::var("TRI_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let base = dirs::home_dir().ok_or_else(|| anyhow!("cannot locate home directory"))?;
    Ok(base.join(".tri"))
}

pub fn wallet_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("wallet.json"))
}

pub fn exists() -> bool {
    wallet_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn save(ks: &Keystore) -> Result<PathBuf> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = wallet_path()?;
    let json = serde_json::to_string_pretty(ks)?;
    std::fs::write(&path, json)?;
    restrict_permissions(&path);
    Ok(path)
}

pub fn load() -> Result<Keystore> {
    let path = wallet_path()?;
    if !path.exists() {
        bail!("no wallet found at {}. Run 'tri new' first.", path.display());
    }
    let data = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

pub fn read_passphrase(prompt: &str) -> Result<String> {
    Ok(rpassword::prompt_password(prompt)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_round_trip() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let ks = Keystore::seal(phrase, "correct horse battery").unwrap();
        assert!(ks.protected);
        assert_eq!(ks.open("correct horse battery").unwrap(), phrase);
        assert!(ks.open("wrong").is_err());
    }

    /// Writes an unprotected wallet at TRI_HOME so the command line can be
    /// exercised without a console.
    #[test]
    #[ignore = "test fixture, writes a wallet file"]
    fn write_unprotected_fixture() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let ks = Keystore::seal(phrase, "").unwrap();
        let path = save(&ks).unwrap();
        println!("wrote {}", path.display());
    }
}
