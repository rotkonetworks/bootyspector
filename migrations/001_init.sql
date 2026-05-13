-- Chain specifications table
CREATE TABLE IF NOT EXISTS chain_specs (
    id INTEGER PRIMARY KEY NOT NULL,
    network TEXT NOT NULL UNIQUE,  -- e.g., "polkadot", "kusama", "asset-hub-polkadot"
    relay_chain TEXT,               -- e.g., "polkadot", "kusama", "paseo", null for relay chains
    protocol_id TEXT NOT NULL,      -- e.g., "dot", "ksm"
    spec_url TEXT NOT NULL,         -- URL to official chain spec
    spec_hash TEXT,                 -- SHA256 of chain spec content
    last_synced TIMESTAMP,
    spec_content TEXT,              -- Compressed JSON of chain spec (without bootnodes)
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Monitored bootnodes table
CREATE TABLE IF NOT EXISTS bootnodes (
    id INTEGER PRIMARY KEY NOT NULL,
    network TEXT NOT NULL,
    operator TEXT,                  -- null for bootnodes from chain specs
    multiaddr TEXT NOT NULL,
    protocol TEXT NOT NULL,         -- "tcp" or "wss"
    source TEXT NOT NULL,           -- "chainspec" or "manual"
    added_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    last_seen TIMESTAMP,            -- last successful test
    UNIQUE(network, multiaddr),
    FOREIGN KEY (network) REFERENCES chain_specs(network)
);

-- Test results history
CREATE TABLE IF NOT EXISTS test_results (
    id INTEGER PRIMARY KEY NOT NULL,
    bootnode_id INTEGER NOT NULL,
    test_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    discovered_peers INTEGER,
    connected_peers INTEGER,
    test_duration_ms INTEGER,
    status TEXT,                    -- "success", "timeout", "insufficient_peers", etc.
    error_details TEXT,
    FOREIGN KEY (bootnode_id) REFERENCES bootnodes(id)
);

-- Indices for common queries
CREATE INDEX IF NOT EXISTS idx_bootnodes_network ON bootnodes(network);
CREATE INDEX IF NOT EXISTS idx_bootnodes_active ON bootnodes(active);
CREATE INDEX IF NOT EXISTS idx_bootnodes_source ON bootnodes(source);
CREATE INDEX IF NOT EXISTS idx_test_results_bootnode ON test_results(bootnode_id);
CREATE INDEX IF NOT EXISTS idx_test_results_time ON test_results(test_time);
CREATE INDEX IF NOT EXISTS idx_test_results_success ON test_results(success);
