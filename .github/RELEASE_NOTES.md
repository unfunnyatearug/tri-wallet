## This is an extremely early build. Using it is a bad idea.

Releases exist for convenience and so that public releases are tracked from the
start. They are not a recommendation to put money in this wallet.

The software has not been audited, it has not been used at scale, and some of
its code paths have never run against the live network. Assume it can lose
funds.

If you use it anyway, use an amount you are prepared to lose entirely, and
write the recovery phrase down on paper before sending anything to it.

## Authorship

This program was mostly written with Claude Code.

## Install

Download `tri-setup.exe` and run it. It installs to
`%LOCALAPPDATA%\Programs\tri`, adds a Start Menu entry, puts `tri` on the user
PATH, and registers an uninstaller. No administrator rights are required and
nothing is written outside the user profile.

Windows SmartScreen will warn about an unrecognised publisher because the
installer is not code signed. Checksums are in `SHA256SUMS.txt`.

`tri.exe` and `tri-gui.exe` are attached for anyone who prefers to place them
manually.

## Reference

Full documentation, design notes, and the current list of limitations are in
the [README](https://github.com/unfunnyatearug/tri-wallet#readme).
