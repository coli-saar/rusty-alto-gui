# Rusty Alto GUI

Rusty Alto is a native Iced workbench for loading interpreted regular tree
grammars, constructing parse charts, enumerating weighted derivations, and
inspecting interpretation values.

## Supported platforms

The first release targets:

- macOS 13 or newer (`.app` and `.dmg`)
- Windows 10 or newer (`.msi`)
- Current x86-64 Linux distributions (`.deb` and `.AppImage`)

Release artifacts are currently unsigned. macOS Gatekeeper and Windows
SmartScreen may therefore require an explicit confirmation before first launch.

## Development setup

Install Rust 1.88 or newer and check out the GUI and core library as siblings:

```text
workspace/
├── rusty-alto/
└── rusty-alto-gui/
```

Then run:

```sh
cd rusty-alto-gui
cargo run --release
```

The GUI deliberately uses `rusty-alto = { path = "../rusty-alto" }` for now.
The compatible core revision used by CI is
`9c3f1a230a2cc9f7329f5da439c17912f977dcb3`.

## Quality checks

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

## Packaging

Install [cargo-packager](https://github.com/crabnebula-dev/cargo-packager):

```sh
cargo install cargo-packager --locked
```

Build native packages on the target operating system:

```sh
# macOS
cargo packager --release --formats app,dmg

# Windows
cargo packager --release --formats msi

# Linux
cargo packager --release --formats deb,appimage
```

Artifacts are written to `dist/`. Cross-platform packaging is not supported:
each installer must be built and smoke-tested on its native operating system.

## Release verification

Before publishing a tag:

1. Run all quality checks.
2. Confirm `cargo tree -i block` reports no matching package.
3. Build the native packages on all three CI runners.
4. Install or open each artifact and load a representative grammar.
5. Verify parsing, cancellation, derivation paging, zoom, and window closure.
6. Review the draft GitHub release before publishing it.

Signing and notarization can be added to the release workflow when platform
credentials are available.

## License

Rusty Alto GUI is licensed under Apache-2.0. The bundled Inter font retains its
own license in `assets/fonts/Inter-LICENSE.txt`.
