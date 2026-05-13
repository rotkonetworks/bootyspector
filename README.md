# bootyspector

## usage

### with default settings
```
cargo run --release
```

### with custom settings
```
cargo run --release -- \
  --node-binary /path/to/polkadot-omni-node \
  --output-dir /custom/output \
  --data-dir /custom/data \
  --chain-spec-dir /custom/specs \
  --max-concurrent 20 \
  --base-port 9700 \
  --timeout 45 \
  --config custom-bootnodes.json \
  --debug
```

## binary requirements

uses polkadot-omni-node (from polkadot-stable2509-1+) which handles all networks with the appropriate chainspecs. the omni-node replaces the need for separate relay chain and parachain binaries.

## prometheus alerting rules

```yaml
groups:
- name: bootnode_alerts
 rules:
 - alert: BootnodeDown
   expr: bootnode_status == 0
   for: 5m
   labels:
     severity: critical
   annotations:
     summary: "Bootnode {{ $labels.provider }}/{{ $labels.network }} is down"
     description: "Bootnode has failed with reason: {{ $labels.failure_reason }}"

 - alert: SlowBootnodeChecks
   expr: bootnode_check_duration_ms > 30000
   for: 5m
   labels:
     severity: warning
   annotations:
     summary: "Slow bootnode checks"
     description: "Check duration > 30s for {{ $labels.provider }}/{{ $labels.network }}"
```
