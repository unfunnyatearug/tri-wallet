# Security

## Status

This is an extremely early build and using it is a bad idea. The published
release exists for convenience and to track public releases from the start, not
as a recommendation to put money in it.

The software has not been audited. It is a personal project. Treat it as
unreviewed code that handles private keys. Several code paths, including
Bitcoin broadcast, have never run against the live network. Assume it can lose
funds.

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
