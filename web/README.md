# Web preview

A browser build of the Flequit UI running on sample data. **It is a layout
preview, not the application**: nothing is persisted, and every action that
would reach storage is inert. See `crates/flequit-web/src/lib.rs` for why.

The useful thing it buys today is checking the responsive layout at a real
phone viewport, in a real browser, without a device.

## Prerequisites

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
```

## Build

```sh
cargo build --release -p flequit-web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript \
  --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/flequit_web.wasm
```

## Serve

The module has to be served over HTTP; opening `index.html` from the filesystem
fails on the WebAssembly MIME type.

```sh
python3 -m http.server --directory web 8080
# then open http://localhost:8080/
```

## Checking it without a wasm toolchain

`flequit-web` is an ordinary workspace member and builds for the host as well,
so `cargo test -j 4 -p flequit-web` exercises the sample data headlessly and
`cargo run` is not needed to know it compiles.

## Not implemented

- Persistence. The browser build is not going to get local storage: the plan is
  that it talks to a Flequit backend server instead, so what is missing here is
  an API client, not an IndexedDB or OPFS repository. The server is not designed
  yet — see `plans/plan.md` section 8.
- Notifications, the file picker, opening URLs. `WebPlatform` reports no
  capabilities, so the UI hides those affordances rather than offering dead
  buttons.
- Language switching writes nothing back, since there is no settings store.
