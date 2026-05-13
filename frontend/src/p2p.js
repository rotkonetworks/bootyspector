import { createLibp2p } from 'libp2p';
import { webSockets } from '@libp2p/websockets';
import { noise } from '@libp2p/noise';
import { yamux } from '@libp2p/yamux';
import { kadDHT } from '@libp2p/kad-dht';
import { identify } from '@libp2p/identify';
import { ping } from '@libp2p/ping';
import { multiaddr } from '@multiformats/multiaddr';

export class P2PClient {
  constructor(chainId, onLog) {
    this.chainId = chainId;
    this.onLog = onLog || (() => {});
    this.node = null;
    this.discoveredPeers = new Set();
    this.connectedPeers = new Set();
  }

  async connect(bootnodeAddr) {
    this.onLog(`initializing libp2p for ${this.chainId}`);

    try {
      // create libp2p node
      this.node = await createLibp2p({
        addresses: {
          listen: []  // browser can't listen
        },
        transports: [
          webSockets()
        ],
        connectionEncrypters: [
          noise()
        ],
        streamMuxers: [
          yamux()
        ],
        services: {
          identify: identify(),
          ping: ping(),
          dht: kadDHT({
            protocol: `/${this.chainId}/kad`,
            clientMode: true  // browser runs in client mode only
          })
        }
      });

      this.onLog(`node created with peer id: ${this.node.peerId.toString()}`);

      // set up event listeners
      this.node.addEventListener('peer:connect', (evt) => {
        const peerId = evt.detail.toString();
        this.onLog(`connected to peer: ${peerId}`);
        this.connectedPeers.add(peerId);
      });

      this.node.addEventListener('peer:disconnect', (evt) => {
        const peerId = evt.detail.toString();
        this.onLog(`disconnected from peer: ${peerId}`);
        this.connectedPeers.delete(peerId);
      });

      this.node.addEventListener('peer:discovery', (evt) => {
        const peerId = evt.detail.id.toString();
        this.onLog(`discovered peer: ${peerId}`);
        this.discoveredPeers.add(peerId);
      });

      // start the node
      await this.node.start();
      this.onLog('libp2p node started');

      // parse and dial bootnode
      const ma = multiaddr(bootnodeAddr);
      this.onLog(`dialing bootnode: ${bootnodeAddr}`);

      const conn = await this.node.dial(ma);
      this.onLog(`connection established to bootnode`);

      return { success: true };
    } catch (error) {
      this.onLog(`error: ${error.message}`);
      throw error;
    }
  }

  async discover(timeoutMs) {
    this.onLog(`starting peer discovery for ${timeoutMs}ms`);

    const startTime = Date.now();

    try {
      // run a DHT query to find peers
      if (this.node.services.dht) {
        this.onLog('querying DHT for peers');

        // query for random peer to trigger peer discovery
        const randomPeerId = this.node.peerId;

        try {
          const peers = await this.node.services.dht.findPeer(randomPeerId);
          this.onLog(`DHT query completed`);
        } catch (err) {
          // DHT queries might fail, that's ok
          this.onLog(`DHT query: ${err.message}`);
        }
      }

      // wait for the timeout period while collecting peers
      await new Promise(resolve => setTimeout(resolve, timeoutMs));

      const duration = Date.now() - startTime;
      this.onLog(`discovery completed in ${duration}ms`);

      return {
        success: true,
        discoveredPeers: this.discoveredPeers.size,
        connectedPeers: this.connectedPeers.size,
        duration_ms: duration
      };
    } catch (error) {
      this.onLog(`discovery error: ${error.message}`);
      throw error;
    }
  }

  async stop() {
    if (this.node) {
      this.onLog('stopping libp2p node');
      await this.node.stop();
      this.node = null;
    }
  }

  getStats() {
    return {
      discoveredPeers: Array.from(this.discoveredPeers),
      connectedPeers: Array.from(this.connectedPeers),
      discoveredCount: this.discoveredPeers.size,
      connectedCount: this.connectedPeers.size
    };
  }
}

export async function testBootnode(chainId, bootnodeAddr, timeoutSecs, onLog) {
  const client = new P2PClient(chainId, onLog);

  try {
    await client.connect(bootnodeAddr);
    const result = await client.discover(timeoutSecs * 1000);
    await client.stop();

    return {
      success: result.connectedPeers > 0 || result.discoveredPeers > 0,
      discovered_peers: result.discoveredPeers,
      connected_peers: result.connectedPeers,
      duration_ms: result.duration_ms,
      error: null
    };
  } catch (error) {
    try {
      await client.stop();
    } catch (e) {}

    return {
      success: false,
      discovered_peers: 0,
      connected_peers: 0,
      duration_ms: 0,
      error: error.message
    };
  }
}
