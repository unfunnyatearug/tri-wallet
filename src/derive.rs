use anyhow::{anyhow, Result};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Address, CompressedPublicKey, Network};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use sha2::Sha512;
use std::str::FromStr;

type HmacSha512 = Hmac<Sha512>;

/// Number of BTC receive addresses the wallet watches.
pub const BTC_GAP: u32 = 20;

pub struct Wallet {
    seed: [u8; 64],
}

pub struct BtcKey {
    pub index: u32,
    pub secret: SecretKey,
    pub public: CompressedPublicKey,
    pub address: Address,
}

impl Wallet {
    pub fn from_mnemonic(mnemonic: &str) -> Result<Wallet> {
        let m = bip39::Mnemonic::parse_normalized(mnemonic.trim())
            .map_err(|e| anyhow!("invalid recovery phrase: {e}"))?;
        Ok(Wallet {
            seed: m.to_seed_normalized(""),
        })
    }

    /// BIP84 native segwit: m/84'/0'/0'/0/index
    pub fn btc_key(&self, index: u32) -> Result<BtcKey> {
        let secp = Secp256k1::new();
        let master = Xpriv::new_master(Network::Bitcoin, &self.seed)?;
        let path = DerivationPath::from_str(&format!("m/84'/0'/0'/0/{index}"))?;
        let child = master.derive_priv(&secp, &path)?;
        let secret = child.private_key;
        let public = CompressedPublicKey(secret.public_key(&secp));
        let address = Address::p2wpkh(&public, Network::Bitcoin);
        Ok(BtcKey {
            index,
            secret,
            public,
            address,
        })
    }

    pub fn btc_keys(&self, count: u32) -> Result<Vec<BtcKey>> {
        (0..count).map(|i| self.btc_key(i)).collect()
    }

    /// SLIP-0010 ed25519, path m/44'/501'/0'/0' (the layout Phantom and
    /// Solflare use, so the same phrase restores in either of them).
    pub fn sol_key(&self) -> Result<SigningKey> {
        let mut mac = HmacSha512::new_from_slice(b"ed25519 seed").unwrap();
        mac.update(&self.seed);
        let i = mac.finalize().into_bytes();
        let mut key = [0u8; 32];
        let mut chain = [0u8; 32];
        key.copy_from_slice(&i[0..32]);
        chain.copy_from_slice(&i[32..64]);

        for index in [44u32, 501, 0, 0] {
            let hardened = index | 0x8000_0000;
            let mut data = Vec::with_capacity(37);
            data.push(0u8);
            data.extend_from_slice(&key);
            data.extend_from_slice(&hardened.to_be_bytes());
            let mut mac = HmacSha512::new_from_slice(&chain).unwrap();
            mac.update(&data);
            let i = mac.finalize().into_bytes();
            key.copy_from_slice(&i[0..32]);
            chain.copy_from_slice(&i[32..64]);
        }
        Ok(SigningKey::from_bytes(&key))
    }

    pub fn sol_pubkey(&self) -> Result<[u8; 32]> {
        Ok(self.sol_key()?.verifying_key().to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // BIP84 reference vector.
    const VECTOR: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn bip84_addresses_match_the_reference_vector() {
        let w = Wallet::from_mnemonic(VECTOR).unwrap();
        assert_eq!(
            w.btc_key(0).unwrap().address.to_string(),
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
        );
        assert_eq!(
            w.btc_key(1).unwrap().address.to_string(),
            "bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g"
        );
    }

    // Cross-checked against an independent SLIP-0010 plus RFC 8032
    // implementation, using the same path Phantom and Solflare use.
    #[test]
    fn slip0010_solana_address_matches_the_reference_implementation() {
        let w = Wallet::from_mnemonic(VECTOR).unwrap();
        assert_eq!(
            bs58::encode(w.sol_pubkey().unwrap()).into_string(),
            "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
        );
    }
}
