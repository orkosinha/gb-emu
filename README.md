# gb-emu

A cycle-accurate Game Boy and Game Boy Color emulator. Targets WebAssembly (via wasm-pack) and native platforms (via C FFI).

## Packages

| Package | Features | Use case |
|---|---|---|
| `@orkosinha/gb-emu` | Core emulator | Production apps |
| `@orkosinha/gb-emu-devtools` | + Introspection API | Debuggers, dev tools |

## Usage

### WASM

```bash
npm install @orkosinha/gb-emu
```

API: [`src/wasm.rs`](https://github.com/orkosinha/gb-emu/blob/main/src/wasm.rs#L31)

### C FFI

```bash
cargo build --features ios --no-default-features --release
# outputs: target/release/libgb_emu.a + include/gb_emu.h
```

API: [`include/gb_emu.h`](https://github.com/orkosinha/gb-emu/blob/main/include/gb_emu.h)

### Rust

```toml
gb-emu = { git = "https://github.com/orkosinha/gb-emu" }
```

API: [`src/core.rs`](https://github.com/orkosinha/gb-emu/blob/main/src/core.rs#L294)

## Release

```bash
mise run setup:secrets      # push NPM_TOKEN to GitHub (first time only)
mise run release -- 0.2.0   # bump version, tag, push → CI publishes both packages
```
