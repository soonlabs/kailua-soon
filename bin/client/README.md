## run offline generator

```bash
cargo run --bin soon-offline-generator -- -o oracle_data -c offline.json execution-cache -v
```

## run offline client

```bash
cargo run --bin kailua-offline -- --offline-cfg offline.json
```