# Contributing to seatwise

Thanks for your interest in seatwise. This document explains how to build and test the project, how the code is laid
out, and what we expect from contributions.

## Build and test

seatwise is one Rust crate at the repository root. The binary is `seat`.

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`./build.sh` builds `./seat` from the working tree (release profile). `./build.sh tag` builds the tag on HEAD into
`dist/` and refuses a working tree with changes or a tag that doesn't match `Cargo.toml`.

CI runs the same checks on Linux, macOS and Windows, plus `cargo audit` and `cargo package`.

Tests never touch your real setup. Every location seatwise uses can be moved with an environment variable
(`SEATWISE_CONFIG`, `SEATWISE_DATA`, `SEATWISE_HOME`, see `src/paths.rs`), and the integration tests in `tests/cli.rs`
run the `seat` binary against temporary folders with fake `claude` and `codex` binaries. Tests only talk to localhost.

## Repository layout

- `src/` has one module per concern; [docs/architecture.md](docs/architecture.md) maps them.
- `src/main.rs` is the command line; `src/lib.rs` lists the modules.
- `skills/` holds the skills `seat skills install` puts into a seat. They are embedded in the binary.
- `tests/cli.rs` has the end-to-end tests.
- `config.example.toml` documents every option. Update it with the code.
- `docs/` has the architecture and the security model.

## Conventions

- Keep pull requests small and focused. One change per pull request is easier to review than several.
- Add tests for every behavior change, at the level a user sees it when you can (`tests/cli.rs`).
- Keep seatwise generic. A feature has to make sense for any set of seats and folders, not one particular setup.
  Defaults change nothing until the user opts in: enforcement is off, link handling is the system default.
- Account identifiers stay on the user's computer. Never print org UUIDs, org names, emails or account ids where an
  agent can read them (hook messages, skill output), never send a token anywhere but the agent vendor's own endpoint,
  and never log or store tokens. New output goes through the redaction in `src/privacy.rs`.
- Write comments in plain, concise English. Explain why the code does something when that is not obvious. Do not
  restate what the code says.
- Do not add a new dependency without discussing it first in an issue.
- Never commit secrets, org UUIDs, emails or private folder names. Use placeholders such as `example.com` and
  `~/code/app` in fixtures and examples.
- Add a line under `## [Unreleased]` in CHANGELOG.md for anything a user would notice.

## Releasing

1. Move the Unreleased entries in CHANGELOG.md under a new `## [x.y.z] - YYYY-MM-DD` heading and set `version` in
   `Cargo.toml` to the same number.
2. Merge that, then tag the commit `vx.y.z` and push the tag. The release workflow checks that the tag, `Cargo.toml` and
   the changelog agree, builds binaries for Linux, macOS and Windows (x86_64 and arm64), and publishes them with
   checksums, build provenance and the changelog section as notes.
3. Publish the same commit to crates.io: with the tag checked out and a clean working tree, run
   `cargo publish --locked`. A published version can't be replaced, only yanked, so check the release first.
