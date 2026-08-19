#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use tri::derive::Wallet;
use tri::util::{format_amount, parse_amount, EARLY_BUILD_SHORT, EARLY_BUILD_WARNING};
use tri::{btc, config, derive, keystore, sol};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 620.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("tri wallet"),
        ..Default::default()
    };
    eframe::run_native(
        "tri wallet",
        options,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

fn setup_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 26.0;
        for (_, font) in style.text_styles.iter_mut() {
            font.size *= 1.1;
        }
    });
}

// ------------------------------------------------------------------- model

#[derive(PartialEq, Clone, Copy)]
enum Asset {
    Btc,
    Sol,
    Usdc,
}

impl Asset {
    fn label(self) -> &'static str {
        match self {
            Asset::Btc => "BTC",
            Asset::Sol => "SOL",
            Asset::Usdc => "USDC",
        }
    }
    fn decimals(self) -> u32 {
        match self {
            Asset::Btc => 8,
            Asset::Sol => 9,
            Asset::Usdc => 6,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Welcome,
    Create,
    Import,
    Unlock,
    Main,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Balances,
    Receive,
    Send,
    Security,
}

#[derive(Default)]
struct Balances {
    btc: Option<Result<u64, String>>,
    sol: Option<Result<u64, String>>,
    usdc: Option<Result<u64, String>>,
}

enum Payload {
    Btc { raw_hex: String },
    Sol { dest: [u8; 32], lamports: u64 },
    Usdc { dest: [u8; 32], amount: u64 },
}

struct Quote {
    lines: Vec<(String, String)>,
    warnings: Vec<String>,
    payload: Payload,
}

enum Msg {
    Btc(Result<u64, String>),
    Sol(Result<u64, String>),
    Usdc(Result<u64, String>),
    ReceiveAddress { address: String, note: String },
    Quote(Result<Quote, String>),
    Sent(Result<String, String>),
}

struct App {
    screen: Screen,
    tab: Tab,

    // Wallet creation and unlocking.
    new_phrase: String,
    phrase_written: bool,
    pass1: String,
    pass2: String,
    import_phrase: String,
    unlock_pass: String,
    error: Option<String>,
    status: Option<String>,

    // Unlocked state. The recovery phrase stays in memory while the window is
    // open, which is what lets it sign without asking again.
    mnemonic: Option<String>,
    protected: bool,
    btc_address: String,
    btc_address_note: String,
    sol_address: String,

    balances: Balances,
    pending_loads: u32,

    send_asset: Asset,
    send_to: String,
    send_amount: String,
    fee_rate: String,
    quote: Option<Quote>,
    working: bool,
    last_signature: Option<String>,

    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    fn new() -> App {
        let (tx, rx) = channel();
        let screen = if keystore::exists() {
            Screen::Unlock
        } else {
            Screen::Welcome
        };
        App {
            screen,
            tab: Tab::Balances,
            new_phrase: String::new(),
            phrase_written: false,
            pass1: String::new(),
            pass2: String::new(),
            import_phrase: String::new(),
            unlock_pass: String::new(),
            error: None,
            status: None,
            mnemonic: None,
            protected: false,
            btc_address: String::new(),
            btc_address_note: String::new(),
            sol_address: String::new(),
            balances: Balances::default(),
            pending_loads: 0,
            send_asset: Asset::Sol,
            send_to: String::new(),
            send_amount: String::new(),
            fee_rate: String::new(),
            quote: None,
            working: false,
            last_signature: None,
            tx,
            rx,
        }
    }

    fn open_wallet(&mut self, mnemonic: String, protected: bool) {
        let wallet = match Wallet::from_mnemonic(&mnemonic) {
            Ok(w) => w,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        self.btc_address = wallet.btc_key(0).unwrap().address.to_string();
        self.btc_address_note = String::new();
        self.sol_address = sol::encode_pubkey(&wallet.sol_pubkey().unwrap());
        self.mnemonic = Some(mnemonic);
        self.protected = protected;
        self.screen = Screen::Main;
        self.error = None;
        self.refresh_balances();
        self.refresh_receive_address();
    }

    /// Each asset loads on its own thread. Bitcoin needs one request per
    /// watched address, so waiting for it would hold up the other two.
    fn refresh_balances(&mut self) {
        let mnemonic = match &self.mnemonic {
            Some(m) => m.clone(),
            None => return,
        };
        self.balances = Balances::default();
        self.pending_loads = 3;
        for (which, load) in [
            (0u8, btc_balance as fn(&str) -> Result<u64, String>),
            (1, sol_balance),
            (2, usdc_balance),
        ] {
            let mnemonic = mnemonic.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = load(&mnemonic);
                let _ = tx.send(match which {
                    0 => Msg::Btc(result),
                    1 => Msg::Sol(result),
                    _ => Msg::Usdc(result),
                });
            });
        }
    }

    fn refresh_receive_address(&mut self) {
        let mnemonic = match &self.mnemonic {
            Some(m) => m.clone(),
            None => return,
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let (address, note) = fresh_btc_address(&mnemonic);
            let _ = tx.send(Msg::ReceiveAddress { address, note });
        });
    }

    fn request_quote(&mut self) {
        let mnemonic = match &self.mnemonic {
            Some(m) => m.clone(),
            None => return,
        };
        self.error = None;
        self.status = None;
        self.working = true;
        let asset = self.send_asset;
        let to = self.send_to.trim().to_string();
        let amount = self.send_amount.trim().to_string();
        let fee_rate = self.fee_rate.trim().to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let q = build_quote(&mnemonic, asset, &to, &amount, &fee_rate);
            let _ = tx.send(Msg::Quote(q));
        });
    }

    fn confirm_send(&mut self) {
        let mnemonic = match &self.mnemonic {
            Some(m) => m.clone(),
            None => return,
        };
        let quote = match self.quote.take() {
            Some(q) => q,
            None => return,
        };
        self.working = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Sent(broadcast(&mnemonic, quote.payload)));
        });
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Btc(r) => {
                    self.balances.btc = Some(r);
                    self.pending_loads = self.pending_loads.saturating_sub(1);
                }
                Msg::Sol(r) => {
                    self.balances.sol = Some(r);
                    self.pending_loads = self.pending_loads.saturating_sub(1);
                }
                Msg::Usdc(r) => {
                    self.balances.usdc = Some(r);
                    self.pending_loads = self.pending_loads.saturating_sub(1);
                }
                Msg::ReceiveAddress { address, note } => {
                    self.btc_address = address;
                    self.btc_address_note = note;
                }
                Msg::Quote(Ok(q)) => {
                    self.quote = Some(q);
                    self.working = false;
                }
                Msg::Quote(Err(e)) => {
                    self.error = Some(e);
                    self.working = false;
                }
                Msg::Sent(Ok(id)) => {
                    self.last_signature = Some(id);
                    self.status = Some("Sent.".to_string());
                    self.working = false;
                    self.send_to.clear();
                    self.send_amount.clear();
                    self.refresh_balances();
                }
                Msg::Sent(Err(e)) => {
                    self.error = Some(e);
                    self.working = false;
                }
            }
        }
    }
}

// ------------------------------------------------------------------ workers

fn wallet_and_config(mnemonic: &str) -> Result<(Wallet, tri::config::Config), String> {
    let wallet = Wallet::from_mnemonic(mnemonic).map_err(|e| e.to_string())?;
    Ok((wallet, config::load()))
}

fn btc_balance(mnemonic: &str) -> Result<u64, String> {
    let (wallet, cfg) = wallet_and_config(mnemonic)?;
    let es = btc::Esplora::new(&cfg.esplora);
    let keys = wallet.btc_keys(derive::BTC_GAP).map_err(|e| e.to_string())?;
    btc::scan_balance(&es, &keys)
        .map(|(confirmed, _)| confirmed)
        .map_err(|e| e.to_string())
}

fn sol_balance(mnemonic: &str) -> Result<u64, String> {
    let (wallet, cfg) = wallet_and_config(mnemonic)?;
    let pk = wallet.sol_pubkey().map_err(|e| e.to_string())?;
    sol::Rpc::new(&cfg.solana_rpc)
        .balance(&pk)
        .map_err(|e| e.to_string())
}

fn usdc_balance(mnemonic: &str) -> Result<u64, String> {
    let (wallet, cfg) = wallet_and_config(mnemonic)?;
    let pk = wallet.sol_pubkey().map_err(|e| e.to_string())?;
    let mint = sol::parse_pubkey(sol::USDC_MINT).map_err(|e| e.to_string())?;
    let ata = sol::associated_token_address(&pk, &mint).map_err(|e| e.to_string())?;
    sol::Rpc::new(&cfg.solana_rpc)
        .token_balance(&ata)
        .map_err(|e| e.to_string())
}

fn fresh_btc_address(mnemonic: &str) -> (String, String) {
    let wallet = match Wallet::from_mnemonic(mnemonic) {
        Ok(w) => w,
        Err(e) => return (String::new(), e.to_string()),
    };
    let cfg = config::load();
    let es = btc::Esplora::new(&cfg.esplora);
    let first = wallet.btc_key(0).unwrap().address.to_string();
    for k in wallet.btc_keys(derive::BTC_GAP).unwrap() {
        match es.tx_count(&k.address.to_string()) {
            Ok(0) => {
                return (
                    k.address.to_string(),
                    "Unused address. Older ones still work.".to_string(),
                )
            }
            Ok(_) => continue,
            Err(_) => {
                return (
                    first,
                    "Could not reach the Bitcoin API, showing the first address.".to_string(),
                )
            }
        }
    }
    (
        first,
        "All watched addresses have been used, so the first one is shown again.".to_string(),
    )
}

fn build_quote(
    mnemonic: &str,
    asset: Asset,
    to: &str,
    amount: &str,
    fee_rate: &str,
) -> Result<Quote, String> {
    if to.is_empty() {
        return Err("enter a destination address".to_string());
    }
    if amount.is_empty() {
        return Err("enter an amount, or the word all".to_string());
    }
    let wallet = Wallet::from_mnemonic(mnemonic).map_err(|e| e.to_string())?;
    let cfg = config::load();
    let sweep = amount.eq_ignore_ascii_case("all");

    match asset {
        Asset::Btc => {
            let dest = btc::parse_address(to).map_err(|e| e.to_string())?;
            let es = btc::Esplora::new(&cfg.esplora);
            let keys = wallet.btc_keys(derive::BTC_GAP).map_err(|e| e.to_string())?;
            let mut owned = Vec::new();
            for (i, k) in keys.iter().enumerate() {
                for u in es.utxos(&k.address.to_string()).map_err(|e| e.to_string())? {
                    owned.push(btc::OwnedUtxo {
                        utxo: u,
                        key_index: i,
                    });
                }
            }
            if owned.is_empty() {
                return Err("no Bitcoin available to spend".to_string());
            }
            let unconfirmed = owned.iter().filter(|o| !o.utxo.status.confirmed).count();
            let rate = if fee_rate.is_empty() {
                es.fee_rate(cfg.fee_target_blocks).map_err(|e| e.to_string())?
            } else {
                fee_rate
                    .parse::<f64>()
                    .map_err(|_| "fee rate must be a number in sat/vB".to_string())?
            };
            let target = if sweep {
                None
            } else {
                Some(parse_amount(amount, 8).map_err(|e| e.to_string())?)
            };
            let plan = btc::plan_send(owned, target, rate).map_err(|e| e.to_string())?;
            let change_address = keys[0].address.clone();
            let (raw_hex, txid) = btc::build_signed_tx(&plan, &keys, &dest, &change_address)
                .map_err(|e| e.to_string())?;

            let mut warnings = Vec::new();
            let ratio = plan.fee as f64 / plan.send_amount.max(1) as f64;
            if ratio > 0.25 {
                warnings.push(format!(
                    "The fee is {:.0} percent of the amount. Bitcoin fees are charged per transaction, not per amount, so small transfers are expensive. A lower fee rate confirms more slowly but costs less.",
                    ratio * 100.0
                ));
            }
            if unconfirmed > 0 {
                warnings.push(format!(
                    "{unconfirmed} incoming payment(s) are still unconfirmed and are being spent here. This will not confirm until they do."
                ));
            }
            Ok(Quote {
                lines: vec![
                    ("Send".into(), format!("{} BTC", format_amount(plan.send_amount, 8))),
                    ("To".into(), dest.to_string()),
                    ("Check".into(), echo(&dest.to_string())),
                    (
                        "Fee".into(),
                        format!(
                            "{} BTC ({} sat at {:.1} sat/vB)",
                            format_amount(plan.fee, 8),
                            plan.fee,
                            rate
                        ),
                    ),
                    ("Inputs".into(), plan.inputs.len().to_string()),
                    ("Change".into(), format!("{} BTC", format_amount(plan.change, 8))),
                    ("Txid".into(), txid.to_string()),
                ],
                warnings,
                payload: Payload::Btc { raw_hex },
            })
        }
        Asset::Sol => {
            let dest = sol::parse_pubkey(to).map_err(|e| e.to_string())?;
            let rpc = sol::Rpc::new(&cfg.solana_rpc);
            let from = wallet.sol_pubkey().map_err(|e| e.to_string())?;
            let balance = rpc.balance(&from).map_err(|e| e.to_string())?;
            let fee = 5_000u64;
            let lamports = if sweep {
                balance.saturating_sub(fee)
            } else {
                parse_amount(amount, 9).map_err(|e| e.to_string())?
            };
            if lamports == 0 {
                return Err("nothing to send".to_string());
            }
            if lamports + fee > balance {
                return Err(format!(
                    "not enough SOL: balance is {}, this needs {}",
                    format_amount(balance, 9),
                    format_amount(lamports + fee, 9)
                ));
            }
            let mut warnings = Vec::new();
            if !rpc.account_exists(&dest).map_err(|e| e.to_string())? {
                warnings.push(
                    "The destination account does not exist on Solana yet. That is normal for a brand new wallet, and it is also what a mistyped address looks like.".to_string(),
                );
            }
            let remaining = balance - lamports - fee;
            if remaining < 1_000_000 {
                warnings.push(
                    "This leaves almost no SOL behind. Future transfers, including USDC transfers, need SOL for fees.".to_string(),
                );
            }
            Ok(Quote {
                lines: vec![
                    ("Send".into(), format!("{} SOL", format_amount(lamports, 9))),
                    ("To".into(), to.to_string()),
                    ("Check".into(), echo(to)),
                    ("Fee".into(), format!("{} SOL", format_amount(fee, 9))),
                ],
                warnings,
                payload: Payload::Sol { dest, lamports },
            })
        }
        Asset::Usdc => {
            let dest = sol::parse_pubkey(to).map_err(|e| e.to_string())?;
            let rpc = sol::Rpc::new(&cfg.solana_rpc);
            let from = wallet.sol_pubkey().map_err(|e| e.to_string())?;
            let mint = sol::parse_pubkey(sol::USDC_MINT).unwrap();
            let src = sol::associated_token_address(&from, &mint).unwrap();
            let balance = rpc.token_balance(&src).map_err(|e| e.to_string())?;
            let units = if sweep {
                balance
            } else {
                parse_amount(amount, 6).map_err(|e| e.to_string())?
            };
            if units == 0 {
                return Err("nothing to send".to_string());
            }
            if units > balance {
                return Err(format!(
                    "not enough USDC: balance is {}",
                    format_amount(balance, 6)
                ));
            }
            let dst = sol::associated_token_address(&dest, &mint).unwrap();
            let needs_account = !rpc.account_exists(&dst).map_err(|e| e.to_string())?;
            let cost = 5_000 + if needs_account { 2_039_280 } else { 0 };
            let sol_balance = rpc.balance(&from).map_err(|e| e.to_string())?;
            if sol_balance < cost {
                return Err(format!(
                    "not enough SOL to pay the fee. Balance is {} SOL, this needs {} SOL. USDC transfers on Solana are paid for in SOL.",
                    format_amount(sol_balance, 9),
                    format_amount(cost, 9)
                ));
            }
            let mut warnings = Vec::new();
            if needs_account {
                warnings.push(
                    "The recipient has no USDC account yet, so this opens one for them. That costs about 0.00204 SOL, refundable to them and not to you.".to_string(),
                );
            }
            if !rpc.account_exists(&dest).map_err(|e| e.to_string())? {
                warnings.push(
                    "The destination wallet does not exist on Solana yet. Confirm the address. USDC sent to a wrong address cannot be recovered.".to_string(),
                );
            }
            Ok(Quote {
                lines: vec![
                    ("Send".into(), format!("{} USDC", format_amount(units, 6))),
                    ("To".into(), to.to_string()),
                    ("Check".into(), echo(to)),
                    ("Fee".into(), format!("{} SOL", format_amount(cost, 9))),
                ],
                warnings,
                payload: Payload::Usdc {
                    dest,
                    amount: units,
                },
            })
        }
    }
}

fn broadcast(mnemonic: &str, payload: Payload) -> Result<String, String> {
    let cfg = config::load();
    match payload {
        Payload::Btc { raw_hex } => btc::Esplora::new(&cfg.esplora)
            .broadcast(&raw_hex)
            .map_err(|e| e.to_string()),
        Payload::Sol { dest, lamports } => {
            let wallet = Wallet::from_mnemonic(mnemonic).map_err(|e| e.to_string())?;
            let key = wallet.sol_key().map_err(|e| e.to_string())?;
            sol::Rpc::new(&cfg.solana_rpc)
                .send_sol(&key, &dest, lamports)
                .map_err(|e| e.to_string())
        }
        Payload::Usdc { dest, amount } => {
            let wallet = Wallet::from_mnemonic(mnemonic).map_err(|e| e.to_string())?;
            let key = wallet.sol_key().map_err(|e| e.to_string())?;
            sol::Rpc::new(&cfg.solana_rpc)
                .send_usdc(&key, &dest, amount)
                .map(|(sig, _)| sig)
                .map_err(|e| e.to_string())
        }
    }
}

fn echo(address: &str) -> String {
    if address.len() <= 12 {
        return address.to_string();
    }
    format!("{}...{}", &address[..6], &address[address.len() - 6..])
}

// --------------------------------------------------------------------- ui

const WARN: egui::Color32 = egui::Color32::from_rgb(214, 138, 42);
const BAD: egui::Color32 = egui::Color32::from_rgb(200, 70, 70);
const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 140, 140);

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain();
        if self.working || self.pending_loads > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::same(22))
            .show(ui, |ui| {
                ui.set_max_width(760.0);
                match self.screen {
                    Screen::Welcome => self.ui_welcome(ui),
                    Screen::Create => self.ui_create(ui),
                    Screen::Import => self.ui_import(ui),
                    Screen::Unlock => self.ui_unlock(ui),
                    Screen::Main => self.ui_main(ui),
                }
            });
    }
}

impl App {
    fn ui_banner(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = self.error.clone() {
            ui.colored_label(BAD, format!("Error: {e}"));
        }
        if let Some(s) = self.status.clone() {
            ui.colored_label(MUTED, s);
        }
    }

    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        ui.add_space(40.0);
        ui.heading("tri wallet");
        ui.label("Bitcoin, Solana and USDC in one recovery phrase.");
        ui.add_space(16.0);
        ui.colored_label(BAD, EARLY_BUILD_WARNING);
        ui.add_space(20.0);
        if ui.button("Create a new wallet").clicked() {
            let mut entropy = [0u8; 16];
            use rand::RngCore;
            rand::thread_rng().fill_bytes(&mut entropy);
            match bip39::Mnemonic::from_entropy(&entropy) {
                Ok(m) => {
                    self.new_phrase = m.to_string();
                    self.phrase_written = false;
                    self.screen = Screen::Create;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        ui.add_space(8.0);
        if ui.button("Restore from a recovery phrase").clicked() {
            self.screen = Screen::Import;
        }
        ui.add_space(16.0);
        self.ui_banner(ui);
    }

    fn ui_passphrase_fields(&mut self, ui: &mut egui::Ui) {
        ui.label("Passphrase");
        ui.add(
            egui::TextEdit::singleline(&mut self.pass1)
                .password(true)
                .desired_width(320.0),
        );
        ui.label("Repeat passphrase");
        ui.add(
            egui::TextEdit::singleline(&mut self.pass2)
                .password(true)
                .desired_width(320.0),
        );
        ui.colored_label(
            MUTED,
            "The passphrase encrypts the wallet file on this computer. It is not part of the recovery phrase and cannot be recovered if lost.",
        );
        if self.pass1.is_empty() {
            ui.colored_label(
                WARN,
                "Leaving this empty is allowed. The wallet file is then readable by anything running as your user, including malware.",
            );
        } else if self.pass1.len() < 8 {
            ui.colored_label(WARN, "Under 8 characters. Short passphrases are guessed quickly.");
        }
    }

    fn save_wallet(&mut self, phrase: String) {
        if self.pass1 != self.pass2 {
            self.error = Some("the two passphrase entries do not match".to_string());
            return;
        }
        let normalized = match bip39::Mnemonic::parse_normalized(phrase.trim()) {
            Ok(m) => m.to_string(),
            Err(e) => {
                self.error = Some(format!("invalid recovery phrase: {e}"));
                return;
            }
        };
        let ks = match keystore::Keystore::seal(&normalized, &self.pass1) {
            Ok(k) => k,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        if let Err(e) = keystore::save(&ks) {
            self.error = Some(e.to_string());
            return;
        }
        let protected = ks.protected;
        self.pass1.clear();
        self.pass2.clear();
        self.open_wallet(normalized, protected);
    }

    fn ui_create(&mut self, ui: &mut egui::Ui) {
        ui.heading("Recovery phrase");
        ui.colored_label(BAD, EARLY_BUILD_WARNING);
        ui.add_space(8.0);
        ui.colored_label(
            WARN,
            "Write these words down on paper, in this order. This is the only copy that can restore the wallet. Anyone who has them owns the funds.",
        );
        ui.add_space(8.0);
        let words: Vec<&str> = self.new_phrase.split_whitespace().collect();
        egui::Grid::new("phrase").spacing([24.0, 6.0]).show(ui, |ui| {
            for row in 0..(words.len() / 3) {
                for col in 0..3 {
                    let i = row * 3 + col;
                    ui.label(format!("{:2}. {}", i + 1, words[i]));
                }
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        if ui.button("Copy phrase").clicked() {
            ui.ctx().copy_text(self.new_phrase.clone());
            self.status = Some(
                "Copied. The clipboard is readable by other programs, so paste it and clear it."
                    .to_string(),
            );
        }
        ui.add_space(12.0);
        ui.checkbox(&mut self.phrase_written, "I have written the phrase down");
        ui.add_space(12.0);
        self.ui_passphrase_fields(ui);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.phrase_written, egui::Button::new("Create wallet"))
                .clicked()
            {
                let phrase = self.new_phrase.clone();
                self.save_wallet(phrase);
            }
            if ui.button("Back").clicked() {
                self.screen = Screen::Welcome;
            }
        });
        ui.add_space(8.0);
        self.ui_banner(ui);
    }

    fn ui_import(&mut self, ui: &mut egui::Ui) {
        ui.heading("Restore a wallet");
        ui.label("Enter the recovery phrase, all words separated by spaces.");
        ui.add(
            egui::TextEdit::multiline(&mut self.import_phrase)
                .desired_width(560.0)
                .desired_rows(3),
        );
        if keystore::exists() {
            ui.colored_label(
                WARN,
                "A wallet file already exists. Restoring replaces it. Make sure you can still restore the current one.",
            );
        }
        ui.add_space(12.0);
        self.ui_passphrase_fields(ui);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Restore").clicked() {
                let phrase = self.import_phrase.clone();
                self.save_wallet(phrase);
            }
            if ui.button("Back").clicked() {
                self.screen = if keystore::exists() {
                    Screen::Unlock
                } else {
                    Screen::Welcome
                };
            }
        });
        ui.add_space(8.0);
        self.ui_banner(ui);
    }

    fn ui_unlock(&mut self, ui: &mut egui::Ui) {
        ui.add_space(40.0);
        ui.heading("Unlock");
        let ks = match keystore::load() {
            Ok(k) => k,
            Err(e) => {
                ui.colored_label(BAD, e.to_string());
                return;
            }
        };
        if !ks.protected {
            ui.colored_label(
                WARN,
                "This wallet has no passphrase. Anyone who can read your disk can spend these funds.",
            );
            if ui.button("Open").clicked() {
                match ks.open("") {
                    Ok(m) => self.open_wallet(m, false),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        } else {
            ui.label("Passphrase");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.unlock_pass)
                    .password(true)
                    .desired_width(320.0),
            );
            let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Unlock").clicked() || submit {
                match ks.open(&self.unlock_pass) {
                    Ok(m) => {
                        self.unlock_pass.clear();
                        self.open_wallet(m, true);
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        ui.add_space(12.0);
        if ui.button("Restore a different wallet").clicked() {
            self.screen = Screen::Import;
        }
        ui.add_space(8.0);
        self.ui_banner(ui);
    }

    fn ui_main(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Balances, "Balances");
            ui.selectable_value(&mut self.tab, Tab::Receive, "Receive");
            ui.selectable_value(&mut self.tab, Tab::Send, "Send");
            ui.selectable_value(&mut self.tab, Tab::Security, "Security");
        });
        ui.separator();
        ui.colored_label(BAD, EARLY_BUILD_SHORT);
        if !self.protected {
            ui.colored_label(
                WARN,
                "This wallet is stored without a passphrase. Anything running as your user can read it.",
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
            Tab::Balances => self.ui_balances(ui),
            Tab::Receive => self.ui_receive(ui),
            Tab::Send => self.ui_send(ui),
            Tab::Security => self.ui_security(ui),
        });
    }

    fn ui_balances(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading("Balances");
            if ui
                .add_enabled(self.pending_loads == 0, egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh_balances();
            }
            if self.pending_loads > 0 {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        egui::Grid::new("balances")
            .min_col_width(90.0)
            .spacing([32.0, 8.0])
            .show(ui, |ui| {
                row(ui, "BTC", &self.balances.btc, 8);
                row(ui, "SOL", &self.balances.sol, 9);
                row(ui, "USDC", &self.balances.usdc, 6);
            });

        if let (Some(Ok(sol_v)), Some(Ok(usdc_v))) = (&self.balances.sol, &self.balances.usdc) {
            if *usdc_v > 0 && *sol_v < 1_000_000 {
                ui.add_space(8.0);
                ui.colored_label(
                    WARN,
                    "This wallet holds USDC but almost no SOL. Solana charges fees in SOL, so the USDC cannot be moved until a small amount of SOL is added.",
                );
            }
        }
        ui.add_space(8.0);
        self.ui_banner(ui);
    }

    fn ui_receive(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Receive");
        ui.add_space(8.0);

        let btc = self.btc_address.clone();
        let sol_addr = self.sol_address.clone();
        self.address_block(ui, "Bitcoin", &btc, &self.btc_address_note.clone());
        ui.add_space(16.0);
        self.address_block(
            ui,
            "Solana and USDC",
            &sol_addr,
            "USDC uses the same address as SOL. Send USDC on the Solana network only.",
        );
        ui.add_space(12.0);
        ui.colored_label(
            WARN,
            "Bitcoin here is base chain only. Lightning payments cannot be received.",
        );
        ui.add_space(8.0);
        self.ui_banner(ui);
    }

    fn address_block(&mut self, ui: &mut egui::Ui, title: &str, address: &str, note: &str) {
        ui.label(egui::RichText::new(title).strong());
        ui.horizontal(|ui| {
            qr(ui, address, 132.0);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(address).monospace()).wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(4.0);
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(address.to_string());
                    self.status = Some("Copied.".to_string());
                }
                if !note.is_empty() {
                    ui.colored_label(MUTED, note);
                }
            });
        });
    }

    fn ui_send(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Send");
        ui.add_space(8.0);

        if let Some(quote) = &self.quote {
            ui.label(egui::RichText::new("Review").strong());
            ui.add_space(6.0);
            // Values are stacked under their labels rather than put in a grid,
            // so a long address wraps across the full width instead of being
            // squeezed into a narrow column.
            for (label, value) in &quote.lines {
                ui.colored_label(MUTED, label);
                ui.add(
                    egui::Label::new(egui::RichText::new(value).monospace())
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
            for w in &quote.warnings {
                ui.colored_label(WARN, w);
            }
            ui.add_space(8.0);
            ui.colored_label(BAD, "This cannot be reversed once sent.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.working, egui::Button::new("Confirm and send"))
                    .clicked()
                {
                    self.confirm_send();
                }
                if ui.button("Cancel").clicked() {
                    self.quote = None;
                }
                if self.working {
                    ui.spinner();
                }
            });
            ui.add_space(8.0);
            self.ui_banner(ui);
            return;
        }

        ui.horizontal(|ui| {
            ui.label("Asset");
            ui.selectable_value(&mut self.send_asset, Asset::Btc, "BTC");
            ui.selectable_value(&mut self.send_asset, Asset::Sol, "SOL");
            ui.selectable_value(&mut self.send_asset, Asset::Usdc, "USDC");
        });
        ui.add_space(6.0);
        ui.label("Destination address");
        ui.add(egui::TextEdit::singleline(&mut self.send_to).desired_width(560.0));
        ui.add_space(6.0);
        ui.label(format!(
            "Amount in {}, or the word all",
            self.send_asset.label()
        ));
        ui.add(egui::TextEdit::singleline(&mut self.send_amount).desired_width(220.0));
        if self.send_asset == Asset::Btc {
            ui.add_space(6.0);
            ui.label("Fee rate in sat/vB, empty to use the network estimate");
            ui.add(egui::TextEdit::singleline(&mut self.fee_rate).desired_width(120.0));
        }
        ui.add_space(4.0);
        ui.colored_label(
            MUTED,
            format!(
                "Up to {} decimal places.",
                self.send_asset.decimals()
            ),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.working, egui::Button::new("Review"))
                .clicked()
            {
                self.request_quote();
            }
            if self.working {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if let Some(sig) = self.last_signature.clone() {
            ui.colored_label(MUTED, format!("Last sent: {sig}"));
        }
        self.ui_banner(ui);
    }

    fn ui_security(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Security");
        ui.colored_label(BAD, EARLY_BUILD_WARNING);
        ui.add_space(8.0);
        ui.colored_label(
            MUTED,
            "These are recommendations. This wallet does not enforce any of them.",
        );
        ui.add_space(8.0);
        for line in [
            "Write the recovery phrase on paper. Anyone holding those words owns the funds, on all three assets, forever. There is no reset.",
            "Do not photograph or screenshot the phrase. Phone galleries sync to cloud accounts, and cloud accounts get breached.",
            "Do not type the phrase into any website, chat, or support agent. No legitimate service will ever ask for it.",
            "Set a passphrase on the wallet file. Without one it is readable by any program running as you.",
            "Send a small test amount first when using a new address.",
            "Verify the first and last six characters of every address you paste. Clipboard swapping malware is common and it targets exactly this.",
            "Keep a small SOL balance. Solana fees, including USDC transfers, are paid in SOL. A USDC only balance cannot move itself.",
            "Bitcoin fees are charged per transaction, not per amount. Moving small amounts of BTC often costs more than the amount is worth.",
        ] {
            ui.label(format!("- {line}"));
            ui.add_space(4.0);
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("Recovery phrase").strong());
        ui.colored_label(
            WARN,
            "Showing this puts the phrase on screen. Do not do it on a shared or recorded display.",
        );
        if ui.button("Show recovery phrase").clicked() {
            self.status = self.mnemonic.clone();
        }
        if ui.button("Hide").clicked() {
            self.status = None;
        }
        ui.add_space(8.0);
        self.ui_banner(ui);
    }
}

fn row(ui: &mut egui::Ui, label: &str, value: &Option<Result<u64, String>>, decimals: u32) {
    ui.label(egui::RichText::new(label).strong());
    match value {
        None => ui.colored_label(MUTED, "loading"),
        Some(Ok(v)) => ui.label(egui::RichText::new(format_amount(*v, decimals)).monospace()),
        Some(Err(e)) => ui.colored_label(BAD, format!("unavailable ({e})")),
    };
    ui.end_row();
}

fn qr(ui: &mut egui::Ui, data: &str, size: f32) {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let width = code.width();
    let colors = code.to_colors();
    let quiet = 2usize;
    let modules = width + quiet * 2;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
    let step = size / modules as f32;
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                let p = egui::pos2(
                    rect.min.x + (x + quiet) as f32 * step,
                    rect.min.y + (y + quiet) as f32 * step,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(p, egui::vec2(step, step)),
                    0.0,
                    egui::Color32::BLACK,
                );
            }
        }
    }
}
