// p2p.rs - lightweight p2p connectivity testing using litep2p
use anyhow::{Context, Result};
use futures::StreamExt;
use litep2p::{
    config::ConfigBuilder,
    crypto::ed25519::Keypair,
    protocol::{
        libp2p::{
            identify::{Config as IdentifyConfig, IdentifyEvent},
            kademlia::{ConfigBuilder as KademliaConfigBuilder, KademliaEvent},
            ping::Config as PingConfig,
        },
        notification::Config as NotificationConfig,
    },
    transport::{
        tcp::config::Config as TcpConfig,
        websocket::config::Config as WebSocketConfig,
    },
    types::multiaddr::{Multiaddr, Protocol},
    Litep2p, Litep2pEvent,
};
use std::{
    collections::HashSet,
    pin::Pin,
    time::{Duration, Instant},
};
use futures::stream::Stream;
use tracing::{debug, info, warn};

const LOG_TARGET: &str = "p2p";

pub struct P2PClient {
    litep2p: Litep2p,
    kad_handle: litep2p::protocol::libp2p::kademlia::KademliaHandle,
    identify_stream: Pin<Box<dyn Stream<Item = IdentifyEvent> + Send>>,
    _block_announces_handle: litep2p::protocol::notification::NotificationHandle,
    start_time: Instant,
    bootnode_peer: litep2p::PeerId,
}

#[derive(Debug, Clone)]
pub struct PeerDiscoveryResult {
    pub discovered_peers: usize,
    pub connected_peers: usize,
    pub duration_ms: u64,
}

impl P2PClient {
    /// create a new p2p client that connects to the given bootnode
    pub async fn new(
        chain_id: &str,
        bootnode_addr: Multiaddr,
    ) -> Result<Self> {
        // generate random keypair for this test
        let keypair = Keypair::generate();

        info!(
            target: LOG_TARGET,
            "initializing p2p client for chain {} with peer id {}",
            chain_id,
            keypair.public().to_peer_id()
        );

        // extract peer id from bootnode address
        let peer_id = extract_peer_id(&bootnode_addr)?;

        // ensure bootnode address has P2p protocol at the end (polkadot-sdk requirement)
        let bootnode_addr_normalized = if bootnode_addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
            bootnode_addr.clone()
        } else {
            bootnode_addr.clone().with(Protocol::P2p(peer_id.into()))
        };

        // prepare known peers for kademlia (polkadot-sdk approach)
        let known_peers: std::collections::HashMap<_, _> =
            vec![(peer_id, vec![bootnode_addr_normalized.clone()])]
            .into_iter()
            .collect();

        // configure kademlia for peer discovery with bootnode as known peer
        let (kad_config, kad_handle) = KademliaConfigBuilder::new()
            .with_protocol_names(vec![
                format!("/{}/kad", chain_id).into(),
            ])
            .with_known_peers(known_peers)
            .build();

        // configure identify protocol
        let (identify_config, identify_stream) = IdentifyConfig::new(
            format!("/{}/1.0.0", chain_id),
            Some("bootyspector/0.1.0".to_string()),
        );

        // configure ping protocol
        let (ping_config, _ping_stream) = PingConfig::default();

        // configure block-announces notification protocol to appear as full node
        let (block_announces_config, block_announces_handle) = NotificationConfig::new(
            format!("/{}/block-announces/1", chain_id).into(),
            1024 * 1024, // 1MB max notification size
            vec![1], // handshake: role = 1 (full node)
            vec![], // no fallback protocol names
            false, // don't auto accept inbound substreams
            4096, // sync channel size
            4096, // async channel size
            false, // don't dial peers automatically
        );

        // configure tcp transport
        let tcp_config = TcpConfig {
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/0".parse().unwrap()],
            ..Default::default()
        };

        // configure websocket transport
        let ws_config = WebSocketConfig {
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/0/ws".parse().unwrap()],
            ..Default::default()
        };

        // build litep2p configuration
        let litep2p_config = ConfigBuilder::new()
            .with_keypair(keypair)
            .with_tcp(tcp_config)
            .with_websocket(ws_config)
            .with_libp2p_kademlia(kad_config)
            .with_libp2p_identify(identify_config)
            .with_libp2p_ping(ping_config)
            .with_notification_protocol(block_announces_config)
            .with_known_addresses(std::iter::once((peer_id, vec![bootnode_addr_normalized.clone()])))
            .build();

        // create litep2p instance
        let mut litep2p = Litep2p::new(litep2p_config)
            .context("failed to create litep2p instance")?;

        info!(
            target: LOG_TARGET,
            "dialing bootnode peer {} at {}",
            peer_id,
            bootnode_addr
        );

        // dial the bootnode by peer id, with a hard timeout. without this,
        // litep2p.dial() blocks on the kernel TCP handshake (default ~75s
        // on Linux for a non-responding host), pegging a tokio worker
        // thread for the full duration. with max-concurrent > 1 and any
        // unreachable bootnodes in the set, the entire runtime starves.
        // 10s is generous: real bootnodes connect within a few hundred ms.
        match tokio::time::timeout(Duration::from_secs(10), litep2p.dial(&peer_id)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow::anyhow!("failed to dial bootnode: {e}")),
            Err(_) => return Err(anyhow::anyhow!("dial timed out after 10s")),
        }

        Ok(Self {
            litep2p,
            kad_handle,
            identify_stream: Box::pin(identify_stream),
            _block_announces_handle: block_announces_handle,
            start_time: Instant::now(),
            bootnode_peer: peer_id,
        })
    }

    /// run peer discovery for up to `timeout`, but exit as soon as we have
    /// `min_peers` connections after `stabilization`. healthy bootnodes
    /// hand back peers in 1-3s; sitting the full timeout for them is what
    /// made the original cycle take hours. broken bootnodes still burn
    /// the full window — they're the ones we actually want to wait on.
    pub async fn discover_peers(
        &mut self,
        timeout: Duration,
        min_peers: usize,
        stabilization: Duration,
    ) -> Result<PeerDiscoveryResult> {
        let mut discovered_peers = HashSet::new();
        let mut connected_peers = HashSet::new();
        let mut ever_connected_peers = HashSet::new();  // track peers that ever connected
        let mut bootnode_connected = false;

        // add bootnode to discovered
        discovered_peers.insert(self.bootnode_peer);

        let deadline = Instant::now() + timeout;
        let stabilization_deadline = Instant::now() + stabilization;

        info!(target: LOG_TARGET, "starting peer discovery for up to {:?} (early exit at {} peers after {:?})", timeout, min_peers, stabilization);

        while Instant::now() < deadline {
            // early exit: we have enough connected peers and the
            // stabilization window has elapsed. don't sit the full timeout
            // when the bootnode has already done its job.
            if ever_connected_peers.len() >= min_peers
                && Instant::now() >= stabilization_deadline
            {
                info!(
                    target: LOG_TARGET,
                    "early exit: {} peers connected (≥ {} threshold) after stabilization",
                    ever_connected_peers.len(),
                    min_peers,
                );
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // check if we should timeout
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                identify_event = self.identify_stream.next() => {
                    if let Some(IdentifyEvent::PeerIdentified {
                        peer,
                        protocol_version,
                        user_agent,
                        supported_protocols,
                        listen_addresses,
                        observed_address,
                    }) = identify_event {
                        info!(target: LOG_TARGET, "✓ peer identified: {}", peer);
                        if let Some(agent) = &user_agent {
                            info!(target: LOG_TARGET, "  agent: {}", agent);
                        }
                        if let Some(proto_ver) = &protocol_version {
                            info!(target: LOG_TARGET, "  protocol: {}", proto_ver);
                        }
                        info!(target: LOG_TARGET, "  supports {} protocols", supported_protocols.len());
                        info!(target: LOG_TARGET, "  listening on {} addresses", listen_addresses.len());
                        for addr in &listen_addresses {
                            debug!(target: LOG_TARGET, "    - {}", addr);
                        }

                        // add to discovered peers
                        discovered_peers.insert(peer);
                    }
                }
                event = self.litep2p.next_event() => {
                    match event {
                        Some(Litep2pEvent::ConnectionEstablished { peer, endpoint }) => {
                            info!(target: LOG_TARGET, "connection established with peer {} at {:?}", peer, endpoint);
                            connected_peers.insert(peer);
                            ever_connected_peers.insert(peer);

                            // if this is the bootnode connection, wait a bit then start DHT query
                            if peer == self.bootnode_peer && !bootnode_connected {
                                bootnode_connected = true;
                                info!(target: LOG_TARGET, "bootnode connected, waiting 2s before DHT query");
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                info!(target: LOG_TARGET, "starting DHT query for random peer");
                                // Query for a random peer ID to discover network peers
                                let random_peer = litep2p::PeerId::random();
                                let _query_id = self.kad_handle.find_node(random_peer).await;
                            }
                        }
                        Some(Litep2pEvent::ConnectionClosed { peer, connection_id: _ }) => {
                            info!(target: LOG_TARGET, "connection closed with peer {}", peer);
                            connected_peers.remove(&peer);
                        }
                        Some(Litep2pEvent::DialFailure { address, error }) => {
                            warn!(target: LOG_TARGET, "dial failure to {}: {:?}", address, error);
                        }
                        None => {
                            debug!(target: LOG_TARGET, "litep2p stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
                kad_event = self.kad_handle.next() => {
                    match kad_event {
                        Some(KademliaEvent::FindNodeSuccess { target, peers, query_id: _ }) => {
                            info!(target: LOG_TARGET, "find_node success for {}, discovered {} peers", target, peers.len());
                            for (peer, addresses) in peers {
                                info!(target: LOG_TARGET, "  discovered peer: {} with {} addresses", peer, addresses.len());
                                for addr in &addresses {
                                    info!(target: LOG_TARGET, "    address: {}", addr);
                                }
                                discovered_peers.insert(peer);
                            }
                        }
                        Some(KademliaEvent::RoutingTableUpdate { peers }) => {
                            info!(target: LOG_TARGET, "routing table updated with {} peers", peers.len());
                            for peer in &peers {
                                info!(target: LOG_TARGET, "  peer added to routing table: {}", peer);
                                discovered_peers.insert(*peer);
                            }
                        }
                        Some(KademliaEvent::IncomingRecord { .. }) => {
                            debug!(target: LOG_TARGET, "received incoming kad record");
                        }
                        None => {
                            debug!(target: LOG_TARGET, "kademlia stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        let duration_ms = self.start_time.elapsed().as_millis() as u64;

        info!(
            target: LOG_TARGET,
            "peer discovery completed: {} discovered, {} connected in {}ms",
            discovered_peers.len(),
            connected_peers.len(),
            duration_ms
        );

        if !discovered_peers.is_empty() {
            info!(target: LOG_TARGET, "discovered peers:");
            for peer in &discovered_peers {
                info!(target: LOG_TARGET, "  - {}", peer);
            }
        }

        if !connected_peers.is_empty() {
            info!(target: LOG_TARGET, "currently connected peers:");
            for peer in &connected_peers {
                info!(target: LOG_TARGET, "  - {}", peer);
            }
        }

        if !ever_connected_peers.is_empty() {
            info!(target: LOG_TARGET, "peers that connected during test:");
            for peer in &ever_connected_peers {
                info!(target: LOG_TARGET, "  - {}", peer);
            }
        }

        Ok(PeerDiscoveryResult {
            discovered_peers: discovered_peers.len(),
            connected_peers: ever_connected_peers.len(),  // use ever_connected instead
            duration_ms,
        })
    }
}

/// extract peer id from a multiaddr
fn extract_peer_id(addr: &Multiaddr) -> Result<litep2p::PeerId> {
    for protocol in addr.iter() {
        if let Protocol::P2p(multihash) = protocol {
            return litep2p::PeerId::from_multihash(multihash)
                .map_err(|e| anyhow::anyhow!("invalid peer id in multiaddr: {:?}", e));
        }
    }
    anyhow::bail!("no peer id found in multiaddr: {}", addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_peer_id() {
        let addr: Multiaddr = "/dns/polkadot.boot.rotko.net/tcp/31001/p2p/12D3KooWPyEvPEXghnMC67Gff6PuZiSvfx3fmziKiPZcGStZ5xff"
            .parse()
            .unwrap();

        let peer_id = extract_peer_id(&addr).unwrap();
        assert_eq!(
            peer_id.to_string(),
            "12D3KooWPyEvPEXghnMC67Gff6PuZiSvfx3fmziKiPZcGStZ5xff"
        );
    }
}
