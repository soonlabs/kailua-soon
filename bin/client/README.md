## run offline client

```bash
cargo build --bin kailua-offline
cp bin/client/src/offline/offline.example.json offline.json
RUST_LOG="debug" target/debug/kailua-offline --offline-cfg offline.json
```