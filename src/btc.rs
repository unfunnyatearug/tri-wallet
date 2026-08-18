use anyhow::{anyhow, bail, Result};
use bitcoin::absolute::LockTime;
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{
    Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use serde::Deserialize;
use std::str::FromStr;

use crate::derive::BtcKey;

pub const DEFAULT_ESPLORA: &str = "https://blockstream.info/api";
/// Outputs below this are rejected by relays as dust.
const DUST_LIMIT: u64 = 546;

#[derive(Deserialize)]
struct Stats {
    funded_txo_sum: u64,
    spent_txo_sum: u64,
    tx_count: u64,
}

#[derive(Deserialize)]
struct AddressInfo {
    chain_stats: Stats,
    mempool_stats: Stats,
}

#[derive(Deserialize, Clone)]
pub struct UtxoStatus {
    pub confirmed: bool,
}

#[derive(Deserialize, Clone)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub status: UtxoStatus,
}

pub struct Balance {
    pub confirmed: u64,
    pub pending: i64,
    pub tx_count: u64,
}

/// Consecutive unused addresses that end a scan. This is the usual BIP44 gap
/// limit, and it keeps a fresh wallet down to a handful of requests instead of
/// one per watched address.
pub const GAP_STOP: u32 = 5;

pub struct Esplora {
    base: String,
}

impl Esplora {
    pub fn new(base: &str) -> Esplora {
        Esplora {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn get_text(&self, path: &str) -> Result<String> {
        ureq::get(&format!("{}{}", self.base, path))
            .call()
            .map_err(|e| anyhow!("Bitcoin API request failed: {e}"))?
            .into_string()
            .map_err(|e| anyhow!("Bitcoin API read failed: {e}"))
    }

    pub fn balance(&self, address: &str) -> Result<Balance> {
        let body = self.get_text(&format!("/address/{address}"))?;
        let info: AddressInfo = serde_json::from_str(&body)?;
        let confirmed = info
            .chain_stats
            .funded_txo_sum
            .saturating_sub(info.chain_stats.spent_txo_sum);
        let pending =
            info.mempool_stats.funded_txo_sum as i64 - info.mempool_stats.spent_txo_sum as i64;
        Ok(Balance {
            confirmed,
            pending,
            tx_count: info.chain_stats.tx_count + info.mempool_stats.tx_count,
        })
    }

    pub fn utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        let body = self.get_text(&format!("/address/{address}/utxo"))?;
        Ok(serde_json::from_str(&body)?)
    }

    pub fn tx_count(&self, address: &str) -> Result<u64> {
        Ok(self.balance(address)?.tx_count)
    }

    /// Fee rate in sat/vB for the given confirmation target, in blocks.
    pub fn fee_rate(&self, target_blocks: u32) -> Result<f64> {
        let body = self.get_text("/fee-estimates")?;
        let map: serde_json::Value = serde_json::from_str(&body)?;
        let rate = map
            .get(target_blocks.to_string())
            .and_then(|v| v.as_f64())
            .unwrap_or(2.0);
        Ok(rate.max(1.0))
    }

    pub fn broadcast(&self, raw_hex: &str) -> Result<String> {
        let resp = ureq::post(&format!("{}/tx", self.base))
            .set("content-type", "text/plain")
            .send_string(raw_hex);
        match resp {
            Ok(r) => Ok(r.into_string()?.trim().to_string()),
            Err(ureq::Error::Status(_, r)) => {
                let msg = r.into_string().unwrap_or_default();
                bail!("the network rejected the transaction: {}", msg.trim())
            }
            Err(e) => bail!("broadcast failed: {e}"),
        }
    }

    pub fn history(&self, address: &str) -> Result<Vec<serde_json::Value>> {
        let body = self.get_text(&format!("/address/{address}/txs"))?;
        Ok(serde_json::from_str(&body)?)
    }
}

/// Total balance across the watched addresses, stopping after GAP_STOP
/// consecutive unused ones. Returns confirmed and pending amounts.
pub fn scan_balance(es: &Esplora, keys: &[BtcKey]) -> Result<(u64, i64)> {
    let mut confirmed = 0u64;
    let mut pending = 0i64;
    let mut unused_run = 0u32;
    for k in keys {
        let b = es.balance(&k.address.to_string())?;
        confirmed += b.confirmed;
        pending += b.pending;
        if b.tx_count == 0 {
            unused_run += 1;
            if unused_run >= GAP_STOP {
                break;
            }
        } else {
            unused_run = 0;
        }
    }
    Ok((confirmed, pending))
}

pub fn parse_address(s: &str) -> Result<bitcoin::Address> {
    let a = bitcoin::Address::from_str(s.trim())
        .map_err(|_| anyhow!("not a valid Bitcoin address: {s}"))?;
    a.require_network(Network::Bitcoin)
        .map_err(|_| anyhow!("that address is not a mainnet Bitcoin address: {s}"))
}

/// Estimated virtual size of a P2WPKH transaction, in vbytes.
fn vsize(inputs: usize, outputs: usize) -> u64 {
    // 10.5 base + 68 per input + 31 per output, rounded up.
    let total = 105 + 680 * inputs as u64 + 310 * outputs as u64;
    total.div_ceil(10)
}

pub struct OwnedUtxo {
    pub utxo: Utxo,
    pub key_index: usize,
}

pub struct PlannedSend {
    pub inputs: Vec<OwnedUtxo>,
    pub send_amount: u64,
    pub change: u64,
    pub fee: u64,
}

/// Largest-first coin selection. `amount` of None means send everything.
pub fn plan_send(
    mut available: Vec<OwnedUtxo>,
    amount: Option<u64>,
    fee_rate: f64,
) -> Result<PlannedSend> {
    available.sort_by(|a, b| b.utxo.value.cmp(&a.utxo.value));
    let total: u64 = available.iter().map(|u| u.utxo.value).sum();
    if total == 0 {
        bail!("this wallet has no spendable coins");
    }

    let fee_for = |n_in: usize, n_out: usize| -> u64 {
        (vsize(n_in, n_out) as f64 * fee_rate).ceil() as u64
    };

    match amount {
        // Sweep: one output, every input.
        None => {
            let fee = fee_for(available.len(), 1);
            if total <= fee + DUST_LIMIT {
                bail!(
                    "balance {} sat does not cover the fee of {} sat",
                    total,
                    fee
                );
            }
            Ok(PlannedSend {
                send_amount: total - fee,
                inputs: available,
                change: 0,
                fee,
            })
        }
        Some(target) => {
            if target < DUST_LIMIT {
                bail!(
                    "{} sat is below the dust limit of {} sat, the network will not relay it",
                    target,
                    DUST_LIMIT
                );
            }
            let mut chosen: Vec<OwnedUtxo> = Vec::new();
            let mut sum = 0u64;
            for u in available {
                sum += u.utxo.value;
                chosen.push(u);
                let fee_with_change = fee_for(chosen.len(), 2);
                if sum >= target + fee_with_change {
                    let change = sum - target - fee_with_change;
                    if change < DUST_LIMIT {
                        // Change would be dust, so drop the change output and
                        // let the leftover go to the fee.
                        let fee_no_change = fee_for(chosen.len(), 1);
                        if sum >= target + fee_no_change {
                            return Ok(PlannedSend {
                                inputs: chosen,
                                send_amount: target,
                                change: 0,
                                fee: sum - target,
                            });
                        }
                        continue;
                    }
                    return Ok(PlannedSend {
                        inputs: chosen,
                        send_amount: target,
                        change,
                        fee: fee_with_change,
                    });
                }
            }
            bail!(
                "not enough funds: have {} sat, need {} sat plus fees",
                sum,
                target
            )
        }
    }
}

/// Build and sign a P2WPKH spend.
pub fn build_signed_tx(
    plan: &PlannedSend,
    keys: &[BtcKey],
    destination: &bitcoin::Address,
    change_address: &bitcoin::Address,
) -> Result<(String, Txid)> {
    let secp = Secp256k1::new();

    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::new(),
        output: Vec::new(),
    };

    for owned in &plan.inputs {
        tx.input.push(TxIn {
            previous_output: OutPoint {
                txid: Txid::from_str(&owned.utxo.txid)?,
                vout: owned.utxo.vout,
            },
            script_sig: ScriptBuf::new(),
            // Signal replace-by-fee, so a stuck transaction can be bumped.
            sequence: Sequence(0xFFFF_FFFD),
            witness: Witness::new(),
        });
    }

    tx.output.push(TxOut {
        value: Amount::from_sat(plan.send_amount),
        script_pubkey: destination.script_pubkey(),
    });
    if plan.change > 0 {
        tx.output.push(TxOut {
            value: Amount::from_sat(plan.change),
            script_pubkey: change_address.script_pubkey(),
        });
    }

    let mut cache = SighashCache::new(tx.clone());
    let mut witnesses = Vec::new();
    for (i, owned) in plan.inputs.iter().enumerate() {
        let key = &keys[owned.key_index];
        // BIP143 signs against the witness script pubkey of the coin.
        let script_pubkey = key.address.script_pubkey();
        let sighash = cache.p2wpkh_signature_hash(
            i,
            &script_pubkey,
            Amount::from_sat(owned.utxo.value),
            EcdsaSighashType::All,
        )?;
        let msg = Message::from(sighash);
        let sig = secp.sign_ecdsa(&msg, &key.secret);
        let sig = bitcoin::ecdsa::Signature {
            signature: sig,
            sighash_type: EcdsaSighashType::All,
        };
        witnesses.push(Witness::p2wpkh(&sig, &key.public.0));
    }
    for (i, w) in witnesses.into_iter().enumerate() {
        tx.input[i].witness = w;
    }

    let hex = bitcoin::consensus::encode::serialize_hex(&tx);
    Ok((hex, tx.compute_txid()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::Wallet;

    const VECTOR: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn fake_utxo(value: u64, key_index: usize) -> OwnedUtxo {
        OwnedUtxo {
            utxo: Utxo {
                txid: "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
                    .to_string(),
                vout: key_index as u32,
                value,
                status: UtxoStatus { confirmed: true },
            },
            key_index,
        }
    }

    /// Signs a spend of synthetic coins and runs it through the same consensus
    /// verifier the network uses.
    #[test]
    fn signed_transactions_pass_consensus_verification() {
        let wallet = Wallet::from_mnemonic(VECTOR).unwrap();
        let keys = wallet.btc_keys(3).unwrap();
        let dest = parse_address("bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g").unwrap();

        let available = vec![fake_utxo(100_000, 0), fake_utxo(60_000, 1)];
        let plan = plan_send(available, Some(120_000), 5.0).unwrap();
        assert_eq!(plan.inputs.len(), 2);
        assert!(plan.change > 0);

        let (hex, _txid) = build_signed_tx(&plan, &keys, &dest, &keys[0].address).unwrap();
        let raw = hex::decode(&hex).unwrap();
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();

        let spent: std::collections::HashMap<OutPoint, TxOut> = plan
            .inputs
            .iter()
            .map(|o| {
                (
                    OutPoint {
                        txid: Txid::from_str(&o.utxo.txid).unwrap(),
                        vout: o.utxo.vout,
                    },
                    TxOut {
                        value: Amount::from_sat(o.utxo.value),
                        script_pubkey: keys[o.key_index].address.script_pubkey(),
                    },
                )
            })
            .collect();
        tx.verify(|point| spent.get(point).cloned())
            .expect("the signed transaction failed consensus verification");
    }

    #[test]
    fn sweeping_spends_every_coin_and_leaves_no_change() {
        let available = vec![fake_utxo(100_000, 0), fake_utxo(60_000, 1)];
        let plan = plan_send(available, None, 3.0).unwrap();
        assert_eq!(plan.change, 0);
        assert_eq!(plan.send_amount + plan.fee, 160_000);
    }

    #[test]
    fn dust_and_shortfalls_are_refused() {
        assert!(plan_send(vec![fake_utxo(100_000, 0)], Some(100), 2.0).is_err());
        assert!(plan_send(vec![fake_utxo(10_000, 0)], Some(50_000), 2.0).is_err());
    }
}
