# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-07

First public release.

### Added

- Steam Guard code generation with a countdown ring and click-to-copy.
- Trade and market confirmations: accept or reject individually or in bulk.
- Account enrolment, re-sign-in, and authenticator removal via revocation code.
- Import of existing `maFiles` from the original Steam Desktop Authenticator.
- Multiple accounts with aliases, private notes, and drag-to-reorder.
- Optional `maFiles` encryption in the original SDA's format: PBKDF2-HMAC-SHA1
  (50 000 rounds) into AES-256-CBC. Aliases and notes are encrypted with them.
- Selectable `maFiles` folder, so an existing SDA vault can be used in place.
- Tray icon: left-click restores the window, right-click lists the accounts and a
  click copies that account's current code. Minimising sends the window to the tray.
- English and Russian interface, following the system language unless set explicitly.
- Optional acrylic window backdrop, showing through the window background and sidebar.
- Optional persona name and avatar from the public Steam profile, without a Web API key.
- Steam server time sync, so codes stay correct when the local clock drifts.
- Application icon generated at build time and embedded as a Win32 resource.

[Unreleased]: ../../compare/v0.1.0...HEAD
[0.1.0]: ../../releases/tag/v0.1.0
