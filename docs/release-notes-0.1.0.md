# Rusty Alto 0.1.0

Rusty Alto is a native desktop application for parsing with multiple grammar
formalisms. This first release provides a common graphical workflow for
loading grammars, parsing one or more inputs, inspecting parse charts, and
exploring weighted derivations across their interpretations.

## Highlights

- Open Alto-compatible interpreted regular tree grammars (`.irtg`) and Tulipac
  tree-adjoining grammars (`.tag`).
- Inspect grammar rules, states, weights, and interpretation homomorphisms.
- Parse inputs with top-down condensed, indexed condensed, or A* algorithms.
- Use zero, SX, or SX + F heuristics with A* parsing.
- Cancel long-running parses while keeping the grammar and application open.
- Inspect and sort the resulting parse chart.
- Enumerate weighted derivations incrementally, including derivations from
  infinite languages.
- View derivation trees and interpretation values as text, trees, or feature
  structures.
- Switch between interpretations, zoom visualizations, and copy evaluated
  values through the available output codecs.
- Open multiple grammars in separate native windows.

Rusty Alto uses interpreted regular tree grammars as a common representation
for different grammar formalisms. Its parsing and grammar support is provided
by [`rusty-alto` 0.2.0](https://crates.io/crates/rusty-alto/0.2.0).

## Downloads

This release provides native packages and standalone executable archives for:

- macOS 13 or newer: `.dmg` and `.tar.gz`
- Windows 10 or newer: `.msi` and `.zip`
- Current x86-64 Linux distributions: `.deb`, `.AppImage`, and `.tar.gz`

The packages are currently unsigned. macOS Gatekeeper or Windows SmartScreen
may therefore ask for confirmation before the first launch.

## Known limitations

- Signing and notarization are not yet configured.
- Native application menus are currently implemented only on macOS.
- Recent files, drag-and-drop opening, and session restoration are not yet
  available.
- Some large labels and large sets of interpretation tabs may overflow the
  available space.

Feedback and bug reports are welcome in the
[GitHub issue tracker](https://github.com/coli-saar/rusty-alto-gui/issues).
