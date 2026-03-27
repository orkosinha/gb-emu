# gb-emu

Game Boy emulator core. Targets WebAssembly (via wasm-pack) and native platforms (via C FFI).

## Usage

### WASM

```bash
npm install @orkosinha/gb-emu
```

API: [`src/wasm.rs`](https://github.com/orkosinha/gb-emu/blob/main/src/wasm.rs)

### C FFI

```bash
cargo build --features ios --no-default-features --release
# outputs: target/release/libgb_emu.a + include/gb_emu.h
```

API: [`include/gb_emu.h`](https://github.com/orkosinha/gb-emu/blob/main/include/gb_emu.h)

### Rust

```toml
gb-emu = { git = "https://github.com/orkosinha/gb-emu", default-features = false }
```

## Release

```bash
cp .env.example .env        # add NPM_TOKEN
mise run setup:secrets      # push token to GitHub
mise run release -- 0.2.0   # tag + push → CI publishes to npm
```
