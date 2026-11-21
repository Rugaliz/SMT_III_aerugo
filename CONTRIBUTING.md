# Contributing

## Ground rules

- **No game content** in the repository: no assets, no extracted files, no original executables or code.
  Tests that need game data read it from your own installation and skip themselves when it isn't
  available.
- Code is MIT-licensed. Dependencies must use compatible permissive licences; `cargo deny check` enforces
  this.
- Keep game logic free of graphics, audio and window code (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)).

## Before every commit

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check        # install with: cargo install cargo-deny
```

## Tests that use your game installation

Set `AERUGO_GAME_DIR` to the game folder (the one containing `smt3hd_Data`):

```sh
AERUGO_GAME_DIR="/path/to/smt3hd" cargo test --workspace
```
