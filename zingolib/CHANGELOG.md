# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `commands`:
  - `MessagesFilterCommand`: For listing `ValueTransfers` containing memos related to an optional `sender` parameter. Returns a `JsonValue` object.
- `lightclient::messages_containing`: used by `MessagesFilterCommand`.
- `tests`:
  - `message_thread` test.

### Changed

- `LightClient::value_transfers::create_send_value_transfers` (in-function definition) -> `ValueTransfers::create_send_value_transfers`
