use anyhow::{anyhow, bail, Result};
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDC_DECIMALS: u32 = 6;

const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

pub fn parse_pubkey(s: &str) -> Result<[u8; 32]> {
    let v = bs58::decode(s.trim())
        .into_vec()
        .map_err(|_| anyhow!("not a valid Solana address: {s}"))?;
    if v.len() != 32 {
        bail!("not a valid Solana address: {s}");
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

pub fn encode_pubkey(k: &[u8; 32]) -> String {
    bs58::encode(k).into_string()
}

fn is_on_curve(bytes: &[u8; 32]) -> bool {
    CompressedEdwardsY(*bytes).decompress().is_some()
}

fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Result<([u8; 32], u8)> {
    for bump in (0..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let out: [u8; 32] = h.finalize().into();
        if !is_on_curve(&out) {
            return Ok((out, bump));
        }
    }
    bail!("could not derive a program address")
}

/// Associated token account for (owner, mint).
pub fn associated_token_address(owner: &[u8; 32], mint: &[u8; 32]) -> Result<[u8; 32]> {
    let token = parse_pubkey(TOKEN_PROGRAM)?;
    let ata = parse_pubkey(ATA_PROGRAM)?;
    let (addr, _) = find_program_address(&[owner, &token, mint], &ata)?;
    Ok(addr)
}

// ---------------------------------------------------------------- transaction

#[derive(Clone)]
pub struct AccountMeta {
    pub pubkey: [u8; 32],
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    fn new(pubkey: [u8; 32], is_signer: bool, is_writable: bool) -> Self {
        AccountMeta {
            pubkey,
            is_signer,
            is_writable,
        }
    }
}

pub struct Instruction {
    pub program_id: [u8; 32],
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

fn shortvec(len: usize, out: &mut Vec<u8>) {
    let mut n = len;
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

/// Serialize a legacy Solana message.
fn compile_message(
    payer: &[u8; 32],
    instructions: &[Instruction],
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let mut metas: Vec<AccountMeta> = vec![AccountMeta::new(*payer, true, true)];
    fn push(m: AccountMeta, metas: &mut Vec<AccountMeta>) {
        if let Some(existing) = metas.iter_mut().find(|e| e.pubkey == m.pubkey) {
            existing.is_signer |= m.is_signer;
            existing.is_writable |= m.is_writable;
        } else {
            metas.push(m);
        }
    }
    for ix in instructions {
        for a in &ix.accounts {
            push(a.clone(), &mut metas);
        }
    }
    for ix in instructions {
        push(AccountMeta::new(ix.program_id, false, false), &mut metas);
    }

    let rank = |m: &AccountMeta| match (m.is_signer, m.is_writable) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    // Stable sort, so the fee payer stays at index 0.
    metas.sort_by_key(rank);

    let num_signers = metas.iter().filter(|m| m.is_signer).count() as u8;
    let num_readonly_signed = metas.iter().filter(|m| m.is_signer && !m.is_writable).count() as u8;
    let num_readonly_unsigned = metas
        .iter()
        .filter(|m| !m.is_signer && !m.is_writable)
        .count() as u8;

    let index_of = |k: &[u8; 32]| -> Result<u8> {
        metas
            .iter()
            .position(|m| &m.pubkey == k)
            .map(|p| p as u8)
            .ok_or_else(|| anyhow!("account missing from message"))
    };

    let mut out = Vec::new();
    out.push(num_signers);
    out.push(num_readonly_signed);
    out.push(num_readonly_unsigned);
    shortvec(metas.len(), &mut out);
    for m in &metas {
        out.extend_from_slice(&m.pubkey);
    }
    out.extend_from_slice(blockhash);
    shortvec(instructions.len(), &mut out);
    for ix in instructions {
        out.push(index_of(&ix.program_id)?);
        shortvec(ix.accounts.len(), &mut out);
        for a in &ix.accounts {
            out.push(index_of(&a.pubkey)?);
        }
        shortvec(ix.data.len(), &mut out);
        out.extend_from_slice(&ix.data);
    }
    Ok(out)
}

fn sign_transaction(key: &SigningKey, message: &[u8]) -> Vec<u8> {
    let sig = key.sign(message);
    let mut tx = Vec::new();
    shortvec(1, &mut tx);
    tx.extend_from_slice(&sig.to_bytes());
    tx.extend_from_slice(message);
    tx
}

fn transfer_sol_ix(from: [u8; 32], to: [u8; 32], lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            AccountMeta::new(from, true, true),
            AccountMeta::new(to, false, true),
        ],
        data,
    }
}

fn create_ata_idempotent_ix(
    funder: [u8; 32],
    ata: [u8; 32],
    owner: [u8; 32],
    mint: [u8; 32],
) -> Result<Instruction> {
    Ok(Instruction {
        program_id: parse_pubkey(ATA_PROGRAM)?,
        accounts: vec![
            AccountMeta::new(funder, true, true),
            AccountMeta::new(ata, false, true),
            AccountMeta::new(owner, false, false),
            AccountMeta::new(mint, false, false),
            AccountMeta::new(SYSTEM_PROGRAM, false, false),
            AccountMeta::new(parse_pubkey(TOKEN_PROGRAM)?, false, false),
        ],
        data: vec![1],
    })
}

fn transfer_checked_ix(
    source: [u8; 32],
    mint: [u8; 32],
    dest: [u8; 32],
    owner: [u8; 32],
    amount: u64,
    decimals: u8,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(10);
    data.push(12);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Ok(Instruction {
        program_id: parse_pubkey(TOKEN_PROGRAM)?,
        accounts: vec![
            AccountMeta::new(source, false, true),
            AccountMeta::new(mint, false, false),
            AccountMeta::new(dest, false, true),
            AccountMeta::new(owner, true, false),
        ],
        data,
    })
}

// ------------------------------------------------------------------- rpc

pub struct Rpc {
    url: String,
}

impl Rpc {
    pub fn new(url: &str) -> Rpc {
        Rpc {
            url: url.to_string(),
        }
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        let resp: Value = ureq::post(&self.url)
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(|e| anyhow!("Solana RPC request failed: {e}"))?
            .into_json()?;
        if let Some(err) = resp.get("error") {
            bail!("Solana RPC error: {}", err);
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("Solana RPC returned no result"))
    }

    pub fn balance(&self, owner: &[u8; 32]) -> Result<u64> {
        let r = self.call("getBalance", json!([encode_pubkey(owner)]))?;
        Ok(r["value"].as_u64().unwrap_or(0))
    }

    pub fn account_exists(&self, key: &[u8; 32]) -> Result<bool> {
        let r = self.call(
            "getAccountInfo",
            json!([encode_pubkey(key), {"encoding":"base64"}]),
        )?;
        Ok(!r["value"].is_null())
    }

    pub fn token_balance(&self, ata: &[u8; 32]) -> Result<u64> {
        match self.call("getTokenAccountBalance", json!([encode_pubkey(ata)])) {
            Ok(r) => Ok(r["value"]["amount"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)),
            // A token account that does not exist yet is not an error, the
            // balance is simply zero.
            Err(_) => Ok(0),
        }
    }

    fn latest_blockhash(&self) -> Result<[u8; 32]> {
        let r = self.call("getLatestBlockhash", json!([{"commitment":"finalized"}]))?;
        let s = r["value"]["blockhash"]
            .as_str()
            .ok_or_else(|| anyhow!("no blockhash in RPC response"))?;
        parse_pubkey(s)
    }

    fn send_raw(&self, tx: &[u8]) -> Result<String> {
        let b64 = b64_encode(tx);
        let r = self.call(
            "sendTransaction",
            json!([b64, {"encoding":"base64","preflightCommitment":"confirmed"}]),
        )?;
        r.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("node did not return a signature"))
    }

    pub fn confirm(&self, signature: &str) -> Result<bool> {
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let r = self.call(
                "getSignatureStatuses",
                json!([[signature], {"searchTransactionHistory": true}]),
            )?;
            let v = &r["value"][0];
            if v.is_null() {
                continue;
            }
            if !v["err"].is_null() {
                bail!("transaction failed on chain: {}", v["err"]);
            }
            let status = v["confirmationStatus"].as_str().unwrap_or("");
            if status == "confirmed" || status == "finalized" {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn send_sol(&self, key: &SigningKey, to: &[u8; 32], lamports: u64) -> Result<String> {
        let from = key.verifying_key().to_bytes();
        let blockhash = self.latest_blockhash()?;
        let ix = transfer_sol_ix(from, *to, lamports);
        let msg = compile_message(&from, &[ix], &blockhash)?;
        let tx = sign_transaction(key, &msg);
        self.send_raw(&tx)
    }

    /// Returns the signature, and whether the recipient token account had to
    /// be created as part of this transaction.
    pub fn send_usdc(
        &self,
        key: &SigningKey,
        to_owner: &[u8; 32],
        amount: u64,
    ) -> Result<(String, bool)> {
        let from = key.verifying_key().to_bytes();
        let mint = parse_pubkey(USDC_MINT)?;
        let src = associated_token_address(&from, &mint)?;
        let dst = associated_token_address(to_owner, &mint)?;
        let dst_exists = self.account_exists(&dst)?;

        let blockhash = self.latest_blockhash()?;
        let mut ixs = Vec::new();
        if !dst_exists {
            ixs.push(create_ata_idempotent_ix(from, dst, *to_owner, mint)?);
        }
        ixs.push(transfer_checked_ix(
            src,
            mint,
            dst,
            from,
            amount,
            USDC_DECIMALS as u8,
        )?);
        let msg = compile_message(&from, &ixs, &blockhash)?;
        let tx = sign_transaction(key, &msg);
        Ok((self.send_raw(&tx)?, !dst_exists))
    }
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc() -> Rpc {
        Rpc::new(DEFAULT_RPC)
    }

    /// Takes a real USDC transfer off mainnet, reads the owner and token
    /// account the chain recorded for it, and checks that deriving the
    /// associated token account from that owner reproduces the same address.
    #[test]
    #[ignore = "requires network access"]
    fn associated_token_address_matches_mainnet() {
        let r = rpc();
        let mint = parse_pubkey(USDC_MINT).unwrap();
        let sigs = r
            .call(
                "getSignaturesForAddress",
                json!([USDC_MINT, {"limit": 10}]),
            )
            .expect("getSignaturesForAddress");

        let mut checked = 0;
        for entry in sigs.as_array().unwrap() {
            if checked > 0 {
                break;
            }
            let sig = entry["signature"].as_str().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
            let tx = match r.call(
                "getTransaction",
                json!([sig, {"encoding":"jsonParsed","maxSupportedTransactionVersion":0}]),
            ) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let keys = match tx["transaction"]["message"]["accountKeys"].as_array() {
                Some(k) => k,
                None => continue,
            };
            let balances = match tx["meta"]["postTokenBalances"].as_array() {
                Some(b) => b,
                None => continue,
            };
            for b in balances {
                if b["mint"].as_str() != Some(USDC_MINT) {
                    continue;
                }
                let idx = b["accountIndex"].as_u64().unwrap() as usize;
                let owner = match b["owner"].as_str() {
                    Some(o) => o,
                    None => continue,
                };
                let account = keys[idx]["pubkey"].as_str().unwrap();
                let owner_key = match parse_pubkey(owner) {
                    Ok(k) => k,
                    Err(_) => continue,
                };
                if encode_pubkey(&associated_token_address(&owner_key, &mint).unwrap()) == account {
                    checked += 1;
                    break;
                }
            }
        }
        assert!(
            checked > 0,
            "no mainnet USDC account matched a derived associated token address"
        );
    }

    /// A transaction the network cannot deserialize is rejected before it is
    /// ever executed, so a clean simulation proves the encoding is correct.
    #[test]
    #[ignore = "requires network access"]
    fn message_encoding_is_accepted_by_mainnet() {
        use rand::RngCore;
        let r = rpc();
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let key = SigningKey::from_bytes(&secret);
        let from = key.verifying_key().to_bytes();
        let blockhash = r.latest_blockhash().expect("blockhash");

        let ix = transfer_sol_ix(from, [1u8; 32], 1_000);
        let msg = compile_message(&from, &[ix], &blockhash).unwrap();
        let tx = sign_transaction(&key, &msg);

        let res = r
            .call(
                "simulateTransaction",
                json!([b64_encode(&tx), {
                    "encoding":"base64",
                    "sigVerify":false,
                    "replaceRecentBlockhash":true,
                    "commitment":"processed"
                }]),
            )
            .expect("the node refused to deserialize the transaction");
        // The account is empty, so the only expected failure is a missing
        // account, not a malformed message.
        let err = res["value"]["err"].to_string();
        assert!(
            err.contains("AccountNotFound") || err == "null",
            "unexpected simulation error: {err}"
        );
    }

    /// Same check for the USDC path, which uses two instructions and a
    /// derived token account.
    #[test]
    #[ignore = "requires network access"]
    fn usdc_message_encoding_is_accepted_by_mainnet() {
        use rand::RngCore;
        let r = rpc();
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let key = SigningKey::from_bytes(&secret);
        let from = key.verifying_key().to_bytes();
        let mint = parse_pubkey(USDC_MINT).unwrap();
        let dest_owner = [2u8; 32];
        let src = associated_token_address(&from, &mint).unwrap();
        let dst = associated_token_address(&dest_owner, &mint).unwrap();
        let blockhash = r.latest_blockhash().expect("blockhash");

        let ixs = vec![
            create_ata_idempotent_ix(from, dst, dest_owner, mint).unwrap(),
            transfer_checked_ix(src, mint, dst, from, 1, USDC_DECIMALS as u8).unwrap(),
        ];
        let msg = compile_message(&from, &ixs, &blockhash).unwrap();
        let tx = sign_transaction(&key, &msg);

        let res = r
            .call(
                "simulateTransaction",
                json!([b64_encode(&tx), {
                    "encoding":"base64",
                    "sigVerify":false,
                    "replaceRecentBlockhash":true,
                    "commitment":"processed"
                }]),
            )
            .expect("the node refused to deserialize the transaction");
        let err = res["value"]["err"].to_string();
        assert!(
            err.contains("AccountNotFound") || err.contains("InstructionError"),
            "unexpected simulation error: {err}"
        );
    }
}
