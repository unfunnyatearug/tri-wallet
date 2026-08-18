use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub solana_rpc: String,
    pub esplora: String,
    /// Confirmation target for Bitcoin fee estimation, in blocks.
    pub fee_target_blocks: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            solana_rpc: crate::sol::DEFAULT_RPC.to_string(),
            esplora: crate::btc::DEFAULT_ESPLORA.to_string(),
            fee_target_blocks: 6,
        }
    }
}

pub fn path() -> Result<PathBuf> {
    Ok(crate::keystore::data_dir()?.join("config.json"))
}

pub fn load() -> Config {
    let p = match path() {
        Ok(p) => p,
        Err(_) => return Config::default(),
    };
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(c: &Config) -> Result<PathBuf> {
    let p = path()?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, serde_json::to_string_pretty(c)?)?;
    Ok(p)
}
