# bootyspector

# With default settings
```
cargo run --release
```

# With custom settings
```
cargo run --release -- \
  --polkadot-binary /path/to/polkadot \
  --parachain-binary /path/to/polkadot-parachain \
  --output-dir /custom/output \
  --data-dir /custom/data \
  --chain-spec-dir /custom/specs \
  --max-concurrent 20 \
  --base-port 9700 \
  --timeout 45 \
  --config custom-bootnodes.json \
  --debug
```
