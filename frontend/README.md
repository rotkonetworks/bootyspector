# bootyspector frontend

minimalistic solidjs frontend for testing polkadot/substrate bootnodes using pure javascript with js-libp2p.

## features

- **pure client-side**: runs entirely in the browser using js-libp2p
- **no backend required**: direct p2p connectivity from browser
- **multi-chain**: supports polkadot, kusama, westend, and paseo networks
- **real-time logs**: see connection events and peer discovery in real-time
- **websocket-only**: uses wss transport (required for browser security)

## prerequisites

- node.js 18+ and npm

## setup

### 1. install dependencies

```bash
npm install
```

### 2. run development server

```bash
npm run dev
```

the app will be available at `http://localhost:5173`

## usage

1. select a chain (polkadot, kusama, westend, or paseo)
2. choose a bootnode address (preset or enter custom - must use /wss!)
3. set timeout (how long to run the test)
4. click "test bootnode"
5. watch the console logs for connection events
6. see results: discovered peers, connected peers, duration

## how it works

### architecture

```
┌─────────────────┐
│  solidjs ui     │
└────────┬────────┘
         │
         │ javascript
         ↓
┌─────────────────┐
│  js-libp2p      │  pure javascript p2p library
│                 │
│  - websockets   │  ← browser-compatible transport
│  - kad-dht      │  ← peer discovery
│  - identify     │  ← peer identification
│  - noise        │  ← encryption
│  - yamux        │  ← multiplexing
└─────────────────┘
```

### browser limitations

- **websocket only**: browsers can't use raw tcp, so bootnodes must support wss
- **client mode only**: browser can dial out but can't accept incoming connections
- **cors restrictions**: some bootnodes may have cors policies that block browser connections
- **limited peer discovery**: browsers can't listen, reducing peer discovery effectiveness

### bootnode format

bootnodes MUST use websocket transport for browser compatibility:

```
/dns/example.com/tcp/30335/wss/p2p/12D3Koo...
```

note the `/wss` component - plain tcp (`/tcp/30333`) won't work in browsers.

## building for production

```bash
npm run build
```

outputs optimized static files to `dist/` directory.

## development notes

- p2p client is in `src/p2p.js`
- uses pure js-libp2p - no rust/wasm needed
- vite auto-reloads on changes
- use browser devtools console to see additional debug info

## troubleshooting

**connection failures:**
- verify bootnode supports websocket transport (`/wss`)
- check bootnode address is correct
- some bootnodes may block browser connections (cors)
- ensure bootnode is actually online and accessible

**peer discovery limited:**
- this is normal - browsers run in client mode only
- try increasing timeout
- peer discovery from browsers is inherently limited vs native nodes

**libp2p errors:**
- check browser console for detailed error messages
- ensure bootnode peer id in multiaddr is correct
- verify chain id matches bootnode's kad protocol

## comparison with native implementation

| feature | browser (js-libp2p) | native (rust/litep2p) |
|---------|---------------------|------------------------|
| transport | websocket only | tcp, websocket, quic |
| mode | client only | full node |
| listen | no | yes |
| peer discovery | limited | full |
| performance | slower | faster |
| deployment | any web server | requires runtime |

## next steps

- add support for custom kad protocols
- implement peer connection persistence
- add metrics visualization
- support for multiple bootnodes simultaneously
- export test results to json/csv
