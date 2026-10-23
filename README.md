# chidev

Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them

## Crates

| crate | role |
|---|---|
| `chidev` | Umbrella: the name users depend on. Re-exports the core. |
| `chidev-core` | Core (SDK): `domain` (pure rules and types), `app` (use cases and their ports), `infra` (adapters with real I/O). |
| `chidev-cli` | The `chidev` command. |

The layer rules are in `arch-lint.toml` and run with `cargo test`. A crate or layer
that does not fit the project is deleted, together with its `arch-lint.toml` scope.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
