use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};
use litep2p::{
    config::ConfigBuilder,
    crypto::ed25519::Keypair,
    protocol::libp2p::kademlia::{ConfigBuilder as KademliaConfigBuilder, KademliaEvent},
    transport::websocket::config::Config as WebSocketConfig,
    types::multiaddr::{Multiaddr, Protocol},
    Litep2p, Litep2pEvent,
};
use futures::StreamExt;
use std::{collections::HashSet, time::Duration};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

#[derive(Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub discovered_peers: usize,
    pub connected_peers: usize,
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn init_tracing() {
    tracing_wasm::set_as_global_default();
}

#[wasm_bindgen]
pub async fn test_bootnode(
    chain_id: String,
    bootnode_addr: String,
    timeout_secs: u32,
) -> Result<JsValue, JsValue> {
    console_log!("testing bootnode: {} for chain: {}", bootnode_addr, chain_id);

    let start = web_sys::window()
        .unwrap()
        .performance()
        .unwrap()
        .now();

    // parse multiaddr
    let addr: Multiaddr = bootnode_addr
        .parse()
        .map_err(|e| JsValue::from_str(&format!("invalid multiaddr: {:?}", e)))?;

    // extract peer id
    let peer_id = extract_peer_id(&addr)
        .map_err(|e| JsValue::from_str(&format!("failed to extract peer id: {}", e)))?;

    console_log!("dialing peer: {}", peer_id);

    // generate keypair
    let keypair = Keypair::generate();

    // configure kademlia
    let (kad_config, mut kad_handle) = KademliaConfigBuilder::new()
        .with_protocol_names(vec![
            format!("/{}/kad", chain_id).into(),
        ])
        .build();

    // configure websocket transport
    let ws_config = WebSocketConfig {
        listen_addresses: vec![],
        ..Default::default()
    };

    // build litep2p config
    let litep2p_config = ConfigBuilder::new()
        .with_keypair(keypair.clone())
        .with_websocket(ws_config)
        .with_libp2p_kademlia(kad_config)
        .with_known_addresses(std::iter::once((peer_id, vec![addr.clone()])))
        .build();

    // create litep2p instance
    let mut litep2p = Litep2p::new(litep2p_config)
        .map_err(|e| JsValue::from_str(&format!("failed to create litep2p: {:?}", e)))?;

    // dial bootnode
    litep2p.dial(&peer_id).await
        .map_err(|e| JsValue::from_str(&format!("failed to dial: {:?}", e)))?;

    console_log!("connection initiated, starting discovery...");

    // start peer discovery
    let local_peer = *litep2p.local_peer_id();
    let _query_id = kad_handle.find_node(local_peer).await;

    let mut discovered_peers = HashSet::new();
    let mut connected_peers = HashSet::new();
    let deadline = Duration::from_secs(timeout_secs as u64);
    let mut elapsed = 0u64;

    loop {
        if elapsed >= deadline.as_millis() as u64 {
            break;
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                elapsed += 100;
            }
            event = litep2p.next_event() => {
                match event {
                    Some(Litep2pEvent::ConnectionEstablished { peer, .. }) => {
                        console_log!("connected to peer: {}", peer);
                        connected_peers.insert(peer);
                    }
                    Some(Litep2pEvent::ConnectionClosed { peer, .. }) => {
                        console_log!("connection closed: {}", peer);
                        connected_peers.remove(&peer);
                    }
                    Some(Litep2pEvent::DialFailure { error, .. }) => {
                        console_log!("dial failure: {:?}", error);
                    }
                    None => break,
                    _ => {}
                }
            }
            kad_event = kad_handle.next() => {
                match kad_event {
                    Some(KademliaEvent::FindNodeSuccess { peers, .. }) => {
                        console_log!("discovered {} peers", peers.len());
                        for (peer, _) in peers {
                            discovered_peers.insert(peer);
                        }
                    }
                    Some(KademliaEvent::RoutingTableUpdate { peers }) => {
                        for peer in peers {
                            discovered_peers.insert(peer);
                        }
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }

    let end = web_sys::window()
        .unwrap()
        .performance()
        .unwrap()
        .now();

    let result = TestResult {
        success: connected_peers.len() > 0 || discovered_peers.len() > 0,
        discovered_peers: discovered_peers.len(),
        connected_peers: connected_peers.len(),
        duration_ms: (end - start) as u64,
        error: None,
    };

    console_log!(
        "test complete: {} discovered, {} connected",
        result.discovered_peers,
        result.connected_peers
    );

    Ok(serde_wasm_bindgen::to_value(&result)?)
}

fn extract_peer_id(addr: &Multiaddr) -> Result<litep2p::PeerId, String> {
    for protocol in addr.iter() {
        if let Protocol::P2p(multihash) = protocol {
            return litep2p::PeerId::from_multihash(multihash)
                .map_err(|_| "invalid peer id".to_string());
        }
    }
    Err("no peer id in multiaddr".to_string())
}
