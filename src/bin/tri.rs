use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use tri::derive::Wallet;
use tri::util::{confirm, format_amount, note, parse_amount, prompt_line, warn};
use tri::{btc, config, derive, keystore, sol};

#[derive(Parser)]
#[command(
    name = "tri",
    version,
    about = "A wallet for Bitcoin, Solana and USDC.",
    long_about = "A wallet for Bitcoin, Solana and USDC. One recovery phrase covers all three. \
Bitcoin runs on the base chain only, there is no Lightning support."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new wallet and print its recovery phrase.
    New {
        /// Number of words in the recovery phrase.
        #[arg(long, default_value_t = 12)]
        words: usize,
    },
    /// Restore a wallet from an existing recovery phrase.
    Import,
    /// Show the addresses that can receive funds.
    Receive {
        /// Show the first 20 Bitcoin addresses instead of one unused address.
        #[arg(long)]
        all: bool,
    },
    /// Show balances for BTC, SOL and USDC.
    Balance,
    /// Send funds. Asset is one of: btc, sol, usdc.
    Send {
        asset: String,
        to: String,
        /// Amount in whole units, or the word "all" to send everything.
        amount: String,
        /// Bitcoin fee rate in sat/vB. Overrides the network estimate.
        #[arg(long)]
        fee_rate: Option<f64>,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Show recent Bitcoin transactions for this wallet.
    History,
    /// Print the recovery phrase. Requires the passphrase.
    Seed,
    /// Print the security checklist.
    Security,
    /// Show or change network settings.
    Config {
        /// Setting to change: solana_rpc, esplora, fee_target_blocks.
        key: Option<String>,
        value: Option<String>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::New { words } => cmd_new(words),
        Command::Import => cmd_import(),
        Command::Receive { all } => cmd_receive(all),
        Command::Balance => cmd_balance(),
        Command::Send {
            asset,
            to,
            amount,
            fee_rate,
            yes,
        } => cmd_send(&asset, &to, &amount, fee_rate, yes),
        Command::History => cmd_history(),
        Command::Seed => cmd_seed(),
        Command::Security => {
            print_security_checklist();
            Ok(())
        }
        Command::Config { key, value } => cmd_config(key, value),
    }
}

// ------------------------------------------------------------------ helpers

fn unlock() -> Result<Wallet> {
    let ks = keystore::load()?;
    let pass = if ks.protected {
        keystore::read_passphrase("Passphrase: ")?
    } else {
        warn("this wallet has no passphrase. Anyone who can read your disk can spend these funds. Run 'tri security' for the fix.");
        String::new()
    };
    let mnemonic = ks.open(&pass)?;
    Wallet::from_mnemonic(&mnemonic)
}

fn new_passphrase() -> Result<String> {
    println!("A passphrase encrypts the wallet file on this computer.");
    println!("It is not part of the recovery phrase and it cannot be recovered if lost.");
    println!("Leaving it empty is allowed. The wallet file is then readable by anything");
    println!("running as your user, including malware. That is a real risk, not a formality.");
    println!();
    loop {
        let a = keystore::read_passphrase("Passphrase (empty for none): ")?;
        if a.is_empty() {
            if confirm("Continue with no passphrase?")? {
                warn("wallet will be stored unencrypted in effect. Set one later with 'tri import' using the same recovery phrase.");
                return Ok(a);
            }
            continue;
        }
        if a.len() < 8 {
            warn("that passphrase is under 8 characters. Short passphrases are guessed quickly.");
            if !confirm("Use it anyway?")? {
                continue;
            }
        }
        let b = keystore::read_passphrase("Repeat passphrase: ")?;
        if a != b {
            println!("The two entries did not match. Try again.");
            continue;
        }
        return Ok(a);
    }
}

fn print_security_checklist() {
    println!("Security checklist");
    println!();
    println!("These are recommendations. This wallet does not enforce any of them.");
    println!();
    println!("1. Write the recovery phrase on paper. Anyone holding those words owns");
    println!("   the funds, on all three assets, forever. There is no reset.");
    println!("2. Do not photograph or screenshot the phrase. Phone galleries sync to");
    println!("   cloud accounts, and cloud accounts get breached.");
    println!("3. Do not type the phrase into any website, chat, or support agent.");
    println!("   No legitimate service will ever ask for it.");
    println!("4. Set a passphrase on the wallet file. Without one, the file at");
    println!("   ~/.tri/wallet.json is readable by any program running as you.");
    println!("5. Send a small test amount first when using a new address.");
    println!("6. Verify the first and last six characters of every address you paste.");
    println!("   Clipboard-swapping malware is common and it targets exactly this.");
    println!("7. Keep a small SOL balance. Solana fees, including USDC transfers, are");
    println!("   paid in SOL. A USDC-only balance cannot move itself.");
    println!("8. Bitcoin fees are charged per transaction, not per amount. Moving small");
    println!("   amounts of BTC often costs more than the amount is worth.");
}

fn short(addr: &str) -> String {
    if addr.len() <= 12 {
        return addr.to_string();
    }
    format!("{}...{}", &addr[..6], &addr[addr.len() - 6..])
}

// ----------------------------------------------------------------- commands

fn cmd_new(words: usize) -> Result<()> {
    if keystore::exists() {
        bail!(
            "a wallet already exists at {}. Move or delete it first.",
            keystore::wallet_path()?.display()
        );
    }
    if words != 12 && words != 24 {
        bail!("word count must be 12 or 24");
    }
    let entropy_bytes = if words == 24 { 32 } else { 16 };
    let mut entropy = vec![0u8; entropy_bytes];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut entropy);
    let mnemonic = bip39::Mnemonic::from_entropy(&entropy)?;
    let phrase = mnemonic.to_string();

    println!("Recovery phrase ({words} words):");
    println!();
    for (i, w) in phrase.split_whitespace().enumerate() {
        println!("  {:2}. {}", i + 1, w);
    }
    println!();
    println!("Write these words down on paper, in this order, before continuing.");
    println!("This is the only copy that can restore the wallet.");
    println!();

    let pass = new_passphrase()?;
    let ks = keystore::Keystore::seal(&phrase, &pass)?;
    let path = keystore::save(&ks)?;
    println!();
    println!("Wallet saved to {}", path.display());

    let wallet = Wallet::from_mnemonic(&phrase)?;
    print_addresses(&wallet)?;
    println!();
    print_security_checklist();
    Ok(())
}

fn cmd_import() -> Result<()> {
    if keystore::exists() {
        let path = keystore::wallet_path()?;
        warn(&format!("a wallet already exists at {}", path.display()));
        if !confirm("Overwrite it? The current wallet file will be replaced.")? {
            println!("Cancelled.");
            return Ok(());
        }
    }
    println!("Enter the recovery phrase, all words on one line, separated by spaces.");
    let phrase = prompt_line("Phrase: ")?;
    let wallet = Wallet::from_mnemonic(&phrase)?;
    let normalized = bip39::Mnemonic::parse_normalized(phrase.trim())?.to_string();

    let pass = new_passphrase()?;
    let ks = keystore::Keystore::seal(&normalized, &pass)?;
    let path = keystore::save(&ks)?;
    println!();
    println!("Wallet saved to {}", path.display());
    print_addresses(&wallet)?;
    Ok(())
}

fn print_addresses(wallet: &Wallet) -> Result<()> {
    let btc0 = wallet.btc_key(0)?;
    let solpk = wallet.sol_pubkey()?;
    println!();
    println!("Bitcoin address       {}", btc0.address);
    println!("Solana address        {}", sol::encode_pubkey(&solpk));
    println!("USDC address          {}", sol::encode_pubkey(&solpk));
    println!();
    println!("USDC is sent to the Solana address. It is the same address.");
    println!("Only send USDC on the Solana network to it. USDC on Ethereum, Base,");
    println!("Polygon or any other chain will not arrive and cannot be recovered.");
    Ok(())
}

fn cmd_receive(all: bool) -> Result<()> {
    let wallet = unlock()?;
    let cfg = config::load();
    let solpk = wallet.sol_pubkey()?;

    if all {
        println!("Bitcoin addresses (all of these belong to this wallet):");
        for k in wallet.btc_keys(derive::BTC_GAP)? {
            println!("  {:2}  {}", k.index, k.address);
        }
    } else {
        let es = btc::Esplora::new(&cfg.esplora);
        let mut chosen = wallet.btc_key(0)?.address.to_string();
        let mut found_unused = false;
        let mut offline = false;
        for k in wallet.btc_keys(derive::BTC_GAP)? {
            match es.tx_count(&k.address.to_string()) {
                Ok(0) => {
                    chosen = k.address.to_string();
                    found_unused = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => {
                    offline = true;
                    break;
                }
            }
        }
        if found_unused {
            note("a fresh Bitcoin address is shown each time. Older addresses still work.");
        } else if offline {
            note("could not reach the Bitcoin API, showing the first address instead.");
        } else {
            note(&format!(
                "all {} watched Bitcoin addresses have been used, so the first one is shown again. Reusing an address links your payments together publicly.",
                derive::BTC_GAP
            ));
        }
        println!("Bitcoin       {chosen}");
    }

    println!("Solana        {}", sol::encode_pubkey(&solpk));
    println!("USDC          {}", sol::encode_pubkey(&solpk));
    println!();
    println!("USDC uses the Solana address. Send USDC on the Solana network only.");
    println!("Bitcoin here is base-chain only. Lightning payments cannot be received.");
    Ok(())
}

fn cmd_balance() -> Result<()> {
    let wallet = unlock()?;
    let cfg = config::load();

    let es = btc::Esplora::new(&cfg.esplora);
    let mut btc_confirmed = 0u64;
    let mut btc_pending = 0i64;
    let mut btc_err = None;
    match btc::scan_balance(&es, &wallet.btc_keys(derive::BTC_GAP)?) {
        Ok((c, p)) => {
            btc_confirmed = c;
            btc_pending = p;
        }
        Err(e) => btc_err = Some(e),
    }

    let rpc = sol::Rpc::new(&cfg.solana_rpc);
    let solpk = wallet.sol_pubkey()?;
    let lamports = rpc.balance(&solpk);
    let mint = sol::parse_pubkey(sol::USDC_MINT)?;
    let ata = sol::associated_token_address(&solpk, &mint)?;
    let usdc = rpc.token_balance(&ata);

    println!("Asset   Balance");
    match btc_err {
        None => {
            let pending = if btc_pending != 0 {
                format!(
                    "  ({}{} BTC pending)",
                    if btc_pending > 0 { "+" } else { "-" },
                    format_amount(btc_pending.unsigned_abs(), 8)
                )
            } else {
                String::new()
            };
            println!(
                "BTC     {}{}",
                format_amount(btc_confirmed, 8),
                pending
            );
        }
        Some(e) => println!("BTC     unavailable ({e})"),
    }
    let sol_value = match &lamports {
        Ok(v) => {
            println!("SOL     {}", format_amount(*v, 9));
            Some(*v)
        }
        Err(e) => {
            println!("SOL     unavailable ({e})");
            None
        }
    };
    let usdc_value = match &usdc {
        Ok(v) => {
            println!("USDC    {}", format_amount(*v, 6));
            Some(*v)
        }
        Err(e) => {
            println!("USDC    unavailable ({e})");
            None
        }
    };

    if let (Some(l), Some(u)) = (sol_value, usdc_value) {
        if u > 0 && l < 1_000_000 {
            warn("this wallet holds USDC but almost no SOL. Solana charges fees in SOL, so the USDC cannot be moved until a small amount of SOL is added.");
        }
    }
    Ok(())
}

fn cmd_history() -> Result<()> {
    let wallet = unlock()?;
    let cfg = config::load();
    let es = btc::Esplora::new(&cfg.esplora);
    let solpk = wallet.sol_pubkey()?;

    println!("Bitcoin transactions:");
    let mut seen = 0;
    for k in wallet.btc_keys(derive::BTC_GAP)? {
        let txs = match es.history(&k.address.to_string()) {
            Ok(t) => t,
            Err(e) => {
                println!("  unavailable ({e})");
                break;
            }
        };
        for tx in txs.iter().take(5) {
            let txid = tx["txid"].as_str().unwrap_or("");
            let confirmed = tx["status"]["confirmed"].as_bool().unwrap_or(false);
            let height = tx["status"]["block_height"].as_u64();
            let state = match (confirmed, height) {
                (true, Some(h)) => format!("block {h}"),
                (true, None) => "confirmed".to_string(),
                _ => "unconfirmed".to_string(),
            };
            println!("  {}  {}", short(txid), state);
            seen += 1;
        }
    }
    if seen == 0 {
        println!("  none");
    }
    println!();
    println!("Solana history is not stored locally. View it at:");
    println!(
        "  https://solscan.io/account/{}",
        sol::encode_pubkey(&solpk)
    );
    Ok(())
}

fn cmd_seed() -> Result<()> {
    let ks = keystore::load()?;
    warn("the recovery phrase is about to be printed to this terminal.");
    println!("Anyone who reads it, now or from your scrollback, can take every coin");
    println!("in this wallet. Do not do this on a shared or recorded screen.");
    if !confirm("Print it?")? {
        println!("Cancelled.");
        return Ok(());
    }
    let pass = if ks.protected {
        keystore::read_passphrase("Passphrase: ")?
    } else {
        String::new()
    };
    let phrase = ks.open(&pass)?;
    println!();
    println!("{phrase}");
    println!();
    note("clear this terminal when you are done.");
    Ok(())
}

fn cmd_config(key: Option<String>, value: Option<String>) -> Result<()> {
    let mut cfg = config::load();
    match (key, value) {
        (None, _) => {
            println!("solana_rpc         {}", cfg.solana_rpc);
            println!("esplora            {}", cfg.esplora);
            println!("fee_target_blocks  {}", cfg.fee_target_blocks);
            println!();
            println!("Change a setting with: tri config <key> <value>");
        }
        (Some(k), None) => bail!("no value given for '{k}'"),
        (Some(k), Some(v)) => {
            match k.as_str() {
                "solana_rpc" => cfg.solana_rpc = v,
                "esplora" => cfg.esplora = v,
                "fee_target_blocks" => cfg.fee_target_blocks = v.parse()?,
                other => bail!("unknown setting '{other}'"),
            }
            let p = config::save(&cfg)?;
            println!("Saved to {}", p.display());
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- send

fn cmd_send(asset: &str, to: &str, amount: &str, fee_rate: Option<f64>, yes: bool) -> Result<()> {
    match asset.to_lowercase().as_str() {
        "btc" | "bitcoin" => send_btc(to, amount, fee_rate, yes),
        "sol" | "solana" => send_sol(to, amount, yes),
        "usdc" => send_usdc(to, amount, yes),
        other => bail!("unknown asset '{other}'. Use btc, sol or usdc."),
    }
}

fn check_address_echo(to: &str) {
    println!("Destination   {to}");
    println!("Check the first and last six characters against the address you were");
    println!("given: {}", short(to));
}

fn send_btc(to: &str, amount: &str, fee_rate: Option<f64>, yes: bool) -> Result<()> {
    let dest = btc::parse_address(to)?;
    let wallet = unlock()?;
    let cfg = config::load();
    let es = btc::Esplora::new(&cfg.esplora);
    let keys = wallet.btc_keys(derive::BTC_GAP)?;

    let mut owned = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        for u in es.utxos(&k.address.to_string())? {
            owned.push(btc::OwnedUtxo {
                utxo: u,
                key_index: i,
            });
        }
    }
    if owned.is_empty() {
        bail!("no Bitcoin available to spend");
    }
    let unconfirmed = owned.iter().filter(|o| !o.utxo.status.confirmed).count();
    if unconfirmed > 0 {
        note(&format!(
            "{unconfirmed} incoming payment(s) are still unconfirmed and are included in this spend. The transaction will not confirm until they do."
        ));
    }

    let rate = match fee_rate {
        Some(r) => r,
        None => es.fee_rate(cfg.fee_target_blocks)?,
    };
    let target = if amount.eq_ignore_ascii_case("all") {
        None
    } else {
        Some(parse_amount(amount, 8)?)
    };
    let plan = btc::plan_send(owned, target, rate)?;

    // Change goes back to the first address, which is always ours.
    let change_address = keys[0].address.clone();
    let (raw_hex, txid) = btc::build_signed_tx(&plan, &keys, &dest, &change_address)?;

    println!();
    println!("Send          {} BTC", format_amount(plan.send_amount, 8));
    check_address_echo(&dest.to_string());
    println!(
        "Fee           {} BTC ({} sat at {:.1} sat/vB)",
        format_amount(plan.fee, 8),
        plan.fee,
        rate
    );
    println!("Inputs        {}", plan.inputs.len());
    if plan.change > 0 {
        println!("Change        {} BTC", format_amount(plan.change, 8));
    }
    println!("Txid          {txid}");

    let ratio = plan.fee as f64 / plan.send_amount.max(1) as f64;
    if ratio > 0.25 {
        warn(&format!(
            "the fee is {:.0}% of the amount being sent. Bitcoin base-chain fees are charged per transaction, so small transfers are expensive. Waiting for a quieter fee period, or sending a larger amount at once, costs less overall.",
            ratio * 100.0
        ));
        println!("A lower fee rate can be forced with --fee-rate, at the cost of a slower");
        println!("confirmation. Below about 1 sat/vB the network may not relay it at all.");
    }
    println!();
    println!("This cannot be reversed once broadcast.");
    if !yes && !confirm("Broadcast this transaction?")? {
        println!("Cancelled. Nothing was sent.");
        return Ok(());
    }

    let id = es.broadcast(&raw_hex)?;
    println!("Broadcast. Txid {id}");
    println!("Track it at https://blockstream.info/tx/{id}");
    Ok(())
}

fn send_sol(to: &str, amount: &str, yes: bool) -> Result<()> {
    let dest = sol::parse_pubkey(to)?;
    let wallet = unlock()?;
    let cfg = config::load();
    let rpc = sol::Rpc::new(&cfg.solana_rpc);
    let key = wallet.sol_key()?;
    let from = wallet.sol_pubkey()?;

    let balance = rpc.balance(&from)?;
    // Solana charges a flat 5000 lamports per signature.
    let fee = 5_000u64;
    let lamports = if amount.eq_ignore_ascii_case("all") {
        balance.saturating_sub(fee)
    } else {
        parse_amount(amount, 9)?
    };
    if lamports == 0 {
        bail!("nothing to send");
    }
    if lamports + fee > balance {
        bail!(
            "not enough SOL: balance is {}, this would need {}",
            format_amount(balance, 9),
            format_amount(lamports + fee, 9)
        );
    }

    println!();
    println!("Send          {} SOL", format_amount(lamports, 9));
    check_address_echo(to);
    println!("Fee           {} SOL", format_amount(fee, 9));

    if !rpc.account_exists(&dest)? {
        warn("the destination account does not exist on Solana yet. That is normal for a brand new wallet, but it also happens when an address is mistyped. Confirm the address is correct.");
    }
    let remaining = balance - lamports - fee;
    if remaining < 1_000_000 && remaining > 0 {
        note("this leaves almost no SOL behind. Future transfers, including USDC transfers, need SOL for fees.");
    }
    println!();
    println!("This cannot be reversed once sent.");
    if !yes && !confirm("Send?")? {
        println!("Cancelled. Nothing was sent.");
        return Ok(());
    }

    let sig = rpc.send_sol(&key, &dest, lamports)?;
    println!("Submitted. Signature {sig}");
    if rpc.confirm(&sig)? {
        println!("Confirmed.");
    } else {
        note(&format!("not confirmed yet. Check https://solscan.io/tx/{sig}"));
    }
    Ok(())
}

fn send_usdc(to: &str, amount: &str, yes: bool) -> Result<()> {
    let dest = sol::parse_pubkey(to)?;
    let wallet = unlock()?;
    let cfg = config::load();
    let rpc = sol::Rpc::new(&cfg.solana_rpc);
    let key = wallet.sol_key()?;
    let from = wallet.sol_pubkey()?;

    let mint = sol::parse_pubkey(sol::USDC_MINT)?;
    let src_ata = sol::associated_token_address(&from, &mint)?;
    let balance = rpc.token_balance(&src_ata)?;
    let amount_units = if amount.eq_ignore_ascii_case("all") {
        balance
    } else {
        parse_amount(amount, 6)?
    };
    if amount_units == 0 {
        bail!("nothing to send");
    }
    if amount_units > balance {
        bail!(
            "not enough USDC: balance is {}",
            format_amount(balance, 6)
        );
    }

    let sol_balance = rpc.balance(&from)?;
    let dst_ata = sol::associated_token_address(&dest, &mint)?;
    let needs_account = !rpc.account_exists(&dst_ata)?;
    // 5000 lamports per signature, plus rent if a token account is opened.
    let cost = 5_000 + if needs_account { 2_039_280 } else { 0 };

    println!();
    println!("Send          {} USDC", format_amount(amount_units, 6));
    check_address_echo(to);
    println!("Fee           {} SOL", format_amount(cost, 9));
    if needs_account {
        println!("This includes a one-time deposit of about 0.00204 SOL to open a USDC");
        println!("account for the recipient. That deposit is refundable to them, not to you.");
    }
    if sol_balance < cost {
        bail!(
            "not enough SOL to pay the fee. Balance is {} SOL, this needs {} SOL. USDC transfers on Solana are paid for in SOL.",
            format_amount(sol_balance, 9),
            format_amount(cost, 9)
        );
    }
    if !rpc.account_exists(&dest)? {
        warn("the destination wallet does not exist on Solana yet. Confirm the address is correct. USDC sent to a wrong address cannot be recovered.");
    }
    println!();
    println!("This cannot be reversed once sent.");
    if !yes && !confirm("Send?")? {
        println!("Cancelled. Nothing was sent.");
        return Ok(());
    }

    let (sig, created) = rpc.send_usdc(&key, &dest, amount_units)?;
    println!("Submitted. Signature {sig}");
    if created {
        println!("A USDC account was opened for the recipient.");
    }
    if rpc.confirm(&sig)? {
        println!("Confirmed.");
    } else {
        note("not confirmed yet. Check the signature on solscan.io");
    }
    Ok(())
}
