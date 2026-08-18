# Security

## Status

This software has not been audited. It is a personal project. Treat it as
unreviewed code that handles private keys, and keep balances small.

## Reporting a vulnerability

Report security issues privately through GitHub Security Advisories, under the
Security tab of this repository. Do not open a public issue for a
vulnerability that could be used to take funds.

Include the version, the platform, and the steps needed to reproduce the
problem.

## Scope

The following are in scope:

- Key derivation, signing, and transaction construction.
- The wallet file format and its encryption.
- Any path where a transaction can be produced that does not match what the
  user was shown before confirming.

The following are out of scope:

- The upstream RPC and Esplora endpoints, which are third party services.
- Local attacks that already require code execution as the user, which the
  wallet file encryption is not designed to stop.
