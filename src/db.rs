// db.rs - database operations for chain specs and bootnodes
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ChainSpec {
    pub id: i64,
    pub network: String,
    pub relay_chain: Option<String>,
    pub protocol_id: String,
    pub spec_url: String,
    pub spec_hash: Option<String>,
    pub last_synced: Option<DateTime<Utc>>,
    pub spec_content: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Bootnode {
    pub id: i64,
    pub network: String,
    pub operator: Option<String>,
    pub multiaddr: String,
    pub protocol: String,
    pub source: String,
    pub added_at: DateTime<Utc>,
    pub active: bool,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct TestResult {
    pub bootnode_id: i64,
    pub test_time: DateTime<Utc>,
    pub success: bool,
    pub discovered_peers: Option<i64>,
    pub connected_peers: Option<i64>,
    pub test_duration_ms: Option<i64>,
    pub status: Option<String>,
    pub error_details: Option<String>,
}

impl Database {
    /// create a new database connection
    pub async fn new(database_url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("failed to connect to database")?;

        // run migrations
        sqlx::query(include_str!("../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .context("failed to run migrations")?;

        Ok(Self { pool })
    }

    /// insert or update a chain spec
    pub async fn upsert_chain_spec(
        &self,
        network: &str,
        relay_chain: Option<&str>,
        protocol_id: &str,
        spec_url: &str,
        spec_content: Option<&str>,
        spec_hash: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            INSERT INTO chain_specs (network, relay_chain, protocol_id, spec_url, spec_content, spec_hash, last_synced)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(network) DO UPDATE SET
                relay_chain = excluded.relay_chain,
                protocol_id = excluded.protocol_id,
                spec_url = excluded.spec_url,
                spec_content = excluded.spec_content,
                spec_hash = excluded.spec_hash,
                last_synced = CURRENT_TIMESTAMP
            RETURNING id
            "#,
            network,
            relay_chain,
            protocol_id,
            spec_url,
            spec_content,
            spec_hash
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to upsert chain spec")?;

        Ok(result.id)
    }

    /// get chain spec by network name
    pub async fn get_chain_spec(&self, network: &str) -> Result<Option<ChainSpec>> {
        let spec = sqlx::query_as!(
            ChainSpec,
            r#"
            SELECT id as "id!: i64", network, relay_chain, protocol_id, spec_url, spec_hash,
                   last_synced as "last_synced: DateTime<Utc>",
                   spec_content, created_at as "created_at!: DateTime<Utc>"
            FROM chain_specs
            WHERE network = ?
            "#,
            network
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to get chain spec")?;

        Ok(spec)
    }

    /// insert or update a bootnode
    pub async fn upsert_bootnode(
        &self,
        network: &str,
        operator: Option<&str>,
        multiaddr: &str,
        protocol: &str,
        source: &str,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            INSERT INTO bootnodes (network, operator, multiaddr, protocol, source)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(network, multiaddr) DO UPDATE SET
                operator = excluded.operator,
                protocol = excluded.protocol,
                source = excluded.source,
                active = TRUE
            RETURNING id
            "#,
            network,
            operator,
            multiaddr,
            protocol,
            source
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to upsert bootnode")?;

        Ok(result.id)
    }

    /// get all active bootnodes for a network
    pub async fn get_active_bootnodes(&self, network: &str) -> Result<Vec<Bootnode>> {
        let bootnodes = sqlx::query_as!(
            Bootnode,
            r#"
            SELECT id as "id!: i64", network, operator, multiaddr, protocol, source,
                   added_at as "added_at!: DateTime<Utc>", active as "active!: bool",
                   last_seen as "last_seen: DateTime<Utc>"
            FROM bootnodes
            WHERE network = ? AND active = TRUE
            ORDER BY operator, multiaddr
            "#,
            network
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to get active bootnodes")?;

        Ok(bootnodes)
    }

    /// get all active bootnodes across all networks
    pub async fn get_all_active_bootnodes(&self) -> Result<Vec<Bootnode>> {
        let bootnodes = sqlx::query_as!(
            Bootnode,
            r#"
            SELECT id as "id!: i64", network, operator, multiaddr, protocol, source,
                   added_at as "added_at!: DateTime<Utc>", active as "active!: bool",
                   last_seen as "last_seen: DateTime<Utc>"
            FROM bootnodes
            WHERE active = TRUE
            ORDER BY network, operator, multiaddr
            "#
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to get all active bootnodes")?;

        Ok(bootnodes)
    }

    /// record a test result
    pub async fn record_test_result(&self, result: &TestResult) -> Result<i64> {
        let test_id = sqlx::query!(
            r#"
            INSERT INTO test_results (
                bootnode_id, test_time, success, discovered_peers,
                connected_peers, test_duration_ms, status, error_details
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#,
            result.bootnode_id,
            result.test_time,
            result.success,
            result.discovered_peers,
            result.connected_peers,
            result.test_duration_ms,
            result.status,
            result.error_details
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to record test result")?;

        // update last_seen if successful
        if result.success {
            sqlx::query!(
                "UPDATE bootnodes SET last_seen = ? WHERE id = ?",
                result.test_time,
                result.bootnode_id
            )
            .execute(&self.pool)
            .await
            .context("failed to update last_seen")?;
        }

        Ok(test_id.id)
    }

    /// get test results for a bootnode
    pub async fn get_test_results(
        &self,
        bootnode_id: i64,
        limit: i64,
    ) -> Result<Vec<TestResult>> {
        let results = sqlx::query_as!(
            TestResult,
            r#"
            SELECT bootnode_id, test_time as "test_time!: DateTime<Utc>", success,
                   discovered_peers, connected_peers, test_duration_ms, status, error_details
            FROM test_results
            WHERE bootnode_id = ?
            ORDER BY test_time DESC
            LIMIT ?
            "#,
            bootnode_id,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to get test results")?;

        Ok(results)
    }

    /// deactivate bootnodes that are no longer in spec file or config
    pub async fn deactivate_missing_bootnodes(
        &self,
        network: &str,
        current_addrs: &[String],
    ) -> Result<u64> {
        let placeholders = current_addrs
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "UPDATE bootnodes SET active = FALSE WHERE network = ? AND multiaddr NOT IN ({})",
            placeholders
        );

        let mut query = sqlx::query(&query).bind(network);
        for addr in current_addrs {
            query = query.bind(addr);
        }

        let result = query
            .execute(&self.pool)
            .await
            .context("failed to deactivate missing bootnodes")?;

        Ok(result.rows_affected())
    }
}
