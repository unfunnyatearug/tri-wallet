# tri

A wallet for Bitcoin, Solana and USDC. One recovery phrase covers all three
assets. There is no account to register, no server operated by this project,
and no browser extension.

The wallet ships as two programs built from the same core: `tri`, a command
line interface, and `tri-gui`, a native window. Both read the same wallet file
and can be used interchangeably.

Bitcoin support is base chain only. Lightning is not supported.

This program WAS mostly written with Claude Code, BTW.
The slop README is staying for now

## Status

**This is an extremely early build. Using it is a bad idea.**

The current release exists for convenience and to track public releases from
the start. It is not a recommendation to put money in it. It has not been
audited, it has not been used at scale, and several of its code paths have
never run against the live network. Assume it can lose funds.

If you use it anyway, use an amount you are prepared to lose entirely, and
write the recovery phrase down on paper before you send anything to it.

See [SECURITY.md](SECURITY.md).

## Install

Download `tri-setup.exe` from the
[latest release](https://github.com/unfunnyatearug/tri-wallet/releases/latest)
and run it. The installer places both programs in
`%LOCALAPPDATA%\Programs\tri`, adds a Start Menu entry for the window, adds the
install directory to the user PATH so that `tri` works from any terminal, and
registers an uninstaller with Windows.

The installer does not require administrator rights and writes nothing outside
the user profile.

Windows SmartScreen will warn about an unrecognised publisher, because the
installer is not code signed. Verify the SHA-256 checksum published with each
release before running it if that matters to you.

To remove the wallet, use Apps and Features, or run `uninstall.exe` from the
install directory. Uninstalling does not delete the wallet file.

## Usage

The window is the simpler starting point. Launch **tri wallet** from the Start
Menu. On first run it offers to create a new wallet or restore an existing
recovery phrase. After that it opens to an unlock screen.

The command line covers the same ground:

| Command | Description |
| --- | --- |
| `tri new` | Creates a wallet and prints the recovery phrase. |
| `tri import` | Restores a wallet from an existing phrase. |
| `tri receive` | Shows the addresses to receive on. |
| `tri receive --all` | Lists every watched Bitcoin address. |
| `tri balance` | Shows BTC, SOL and USDC balances. |
| `tri send <asset> <to> <amount>` | Sends funds. Asset is `btc`, `sol` or `usdc`. |
| `tri history` | Recent Bitcoin transactions. |
| `tri seed` | Prints the recovery phrase. |
| `tri security` | Prints the security checklist. |
| `tri config` | Shows or changes the network endpoints. |

`amount` takes a plain decimal, or the word `all` to send the entire balance.
`tri send btc` also accepts `--fee-rate <sat/vB>` to override the network
estimate. Every send accepts `--yes` to skip the confirmation prompt.

## Design

The wallet derives every key from a single BIP39 recovery phrase.

- **Bitcoin** uses BIP84 at `m/84'/0'/0'/0/i`, native segwit, with twenty
  addresses watched. Balance scanning stops after five consecutive unused
  addresses, which is the usual BIP44 gap limit.
- **Solana** uses SLIP-0010 at `m/44'/501'/0'/0'`. This is the path Phantom
  and Solflare use, so the same phrase restores in either of them.
- **USDC** is the Solana SPL token, received at the same address as SOL.
  Transfers use `TransferChecked` and open the recipient token account when it
  does not already exist.

Bitcoin data comes from an Esplora HTTP API, `blockstream.info` by default.
Solana data comes from a JSON-RPC endpoint, the public mainnet endpoint by
default. Neither is operated by this project, and both can be replaced:

```
tri config esplora https://your-esplora-host/api
tri config solana_rpc https://your-rpc-host
```

The public Solana endpoint is heavily rate limited. Anyone using this wallet
regularly should point it at their own endpoint.

There is no local chain data and no indexer. The wallet holds one file.

## The wallet file

The wallet is a single JSON file containing the recovery phrase encrypted with
XChaCha20-Poly1305 under a key derived by Argon2id. It lives at `~/.tri/wallet.json`.

Setting the environment variable `TRI_HOME` moves the directory, which is
useful for keeping the wallet on removable storage:

```
$env:TRI_HOME = "E:\wallet"
```

An empty passphrase is permitted. The file is still written in the same format,
but it is marked as unprotected and every command that opens it prints a
warning. An unprotected wallet offers no defence against anything that can read
your disk.

## Protections

The wallet warns and explains. It does not block, and it does not refuse to
run because a setting is not ideal. The reasoning is that a beginner holding a
small balance needs to understand the risk, not be prevented from acting on it.

- Every send shows the amount, the destination, the fee, and the first and last
  six characters of the address before asking for confirmation.
- A Bitcoin fee above 25 percent of the amount being sent is called out, with
  the reason and the option to lower the fee rate.
- Sending USDC without enough SOL to cover the fee is caught before signing.
  Solana fees are paid in SOL, so a USDC only balance cannot move itself.
- Sending to a Solana address that does not yet exist is flagged, because that
  is also what a mistyped address looks like.
- Opening a token account for a recipient is priced and explained before it
  happens.
- A wallet stored without a passphrase warns on every command that uses it.
- Printing the recovery phrase warns first and asks for confirmation.

Screenshots are not blocked. Clipboard use is not blocked. Both are left to the
user, with the risks stated in `tri security`.

## Build from source

Requires a Rust toolchain and a linker.

```
cargo build --release                 # tri only
cargo build --release --features gui  # tri and tri-gui
```

The graphical interface is behind the `gui` feature so that a command line only
build does not pull in a windowing stack.

On a Windows machine without Visual Studio, install MinGW-w64 and build with
the GNU toolchain:

```
cargo +stable-x86_64-pc-windows-gnu build --release --features gui
```

Building the installer additionally requires NSIS:

```
makensis installer\tri.nsi
```

## Tests

```
cargo test --release
```

The offline suite covers:

- Bitcoin addresses against the BIP84 reference vector.
- The Solana address against an independent SLIP-0010 and RFC 8032
  implementation.
- Wallet file encryption round trips, including rejection of a wrong
  passphrase.
- Coin selection, including sweeps, dust refusal, and insufficient funds.
- A signed Bitcoin transaction, verified with the same consensus code the
  network runs.

Network tests are marked ignored because they depend on public endpoints. They
confirm that associated token account derivation matches real mainnet data, and
that mainnet accepts the transaction encoding for both the SOL and USDC paths:

```
cargo test --release -- --ignored --test-threads=1
```

The public RPC rate limits aggressively, so run them one at a time.

## Limitations

- Bitcoin is base chain only. Lightning is not supported and no Lightning
  support is planned.
- Solana transaction history is not shown in the wallet. The command line
  prints a block explorer link instead.
- Bitcoin change returns to the first address rather than a dedicated change
  chain, which is simpler to reason about but links transactions together.
- Only the first Solana account of the phrase is used.
- Windows is the only platform with a published build. The code has no Windows
  specific logic and should build elsewhere, but that is untested.

## License

MIT. See [LICENSE](LICENSE).
