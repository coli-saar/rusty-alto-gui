# Rusty Alto Iced workbench

This is the single-window Iced redesign of the Rusty Alto GUI. It is kept in a
separate crate so the existing Slint prototype remains available as a reference.

```sh
cd iced
cargo run --release
```

The workbench delegates grammar loading, parsing, chart construction,
derivation enumeration, and interpretation rendering to `rusty-alto`. The UI
keeps grammars and automata behind `Arc` and only owns lightweight table and
tree-layout projections.

