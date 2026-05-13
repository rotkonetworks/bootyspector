import { createSignal, Show } from 'solid-js';
import { testBootnode } from './p2p';

// polkadot brand colors
const colors = {
  pink: '#E6007A',
  cyan: '#07FFFF',
  lime: '#E4FF07',
  violet: '#7916F3',
  white: '#FFFFFF',
  black: '#000000',
  storm700: '#6E7391',
  storm400: '#AEB7CB'
};

function App() {
  const [bootnodeAddr, setBootnodeAddr] = createSignal('');
  const [testing, setTesting] = createSignal(false);
  const [result, setResult] = createSignal(null);
  const [chainId, setChainId] = createSignal('polkadot');

  const runTest = async () => {
    if (testing() || !bootnodeAddr().trim()) return;

    setTesting(true);
    setResult(null);

    try {
      const testResult = await testBootnode(
        chainId(),
        bootnodeAddr().trim(),
        20, // 20 second timeout
        () => {} // no logging
      );

      setResult(testResult);
    } catch (error) {
      setResult({
        success: false,
        connected_peers: 0,
        discovered_peers: 0,
        error: error.toString()
      });
    } finally {
      setTesting(false);
    }
  };

  // determine background color based on results
  const backgroundColor = () => {
    if (!result()) return colors.black;
    const peers = result().connected_peers + result().discovered_peers;
    return peers >= 2 ? colors.lime : colors.violet;
  };

  const textColor = () => {
    if (!result()) return colors.white;
    const peers = result().connected_peers + result().discovered_peers;
    return peers >= 2 ? colors.black : colors.white;
  };

  return (
    <div style={{
      "min-height": "100vh",
      "background": backgroundColor(),
      "transition": "background 0.5s ease",
      "display": "flex",
      "align-items": "center",
      "justify-content": "center",
      "padding": "2rem",
      "font-family": "'Inter', system-ui, sans-serif"
    }}>
      <div style={{
        "max-width": "600px",
        "width": "100%"
      }}>
        <Show when={!result()}>
          {/* input state */}
          <div style={{
            "text-align": "center"
          }}>
            <h1 style={{
              "font-size": "4rem",
              "margin": "0 0 2rem 0",
              "color": colors.white,
              "font-weight": "900",
              "letter-spacing": "-0.03em"
            }}>
              bootyspector
            </h1>

            <div style={{
              "background": "rgba(255, 255, 255, 0.1)",
              "backdrop-filter": "blur(10px)",
              "border-radius": "16px",
              "padding": "2rem",
              "border": `2px solid ${colors.storm400}`
            }}>
              <label style={{
                "display": "block",
                "color": colors.white,
                "font-size": "0.9rem",
                "margin-bottom": "0.5rem",
                "text-align": "left",
                "font-weight": "600"
              }}>
                bootnode address
              </label>

              <input
                type="text"
                value={bootnodeAddr()}
                onInput={(e) => setBootnodeAddr(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && runTest()}
                placeholder="/dns/example.com/tcp/30335/wss/p2p/12D3Koo..."
                disabled={testing()}
                style={{
                  "width": "100%",
                  "padding": "1rem",
                  "font-size": "0.95rem",
                  "font-family": "'Fira Code', monospace",
                  "border": "none",
                  "border-radius": "8px",
                  "background": colors.white,
                  "color": colors.black,
                  "box-sizing": "border-box",
                  "margin-bottom": "1rem"
                }}
                autofocus
              />

              <button
                onClick={runTest}
                disabled={testing() || !bootnodeAddr().trim()}
                style={{
                  "width": "100%",
                  "padding": "1rem",
                  "font-size": "1.1rem",
                  "font-weight": "700",
                  "border": "none",
                  "border-radius": "8px",
                  "background": testing() || !bootnodeAddr().trim() ? colors.storm400 : colors.pink,
                  "color": colors.white,
                  "cursor": testing() || !bootnodeAddr().trim() ? "not-allowed" : "pointer",
                  "transition": "all 0.2s",
                  "text-transform": "uppercase",
                  "letter-spacing": "0.05em"
                }}
              >
                {testing() ? "testing..." : "test bootnode"}
              </button>
            </div>

            <p style={{
              "color": colors.storm400,
              "font-size": "0.85rem",
              "margin-top": "1.5rem"
            }}>
              paste a bootnode address and press enter to test connectivity
            </p>
          </div>
        </Show>

        <Show when={result()}>
          {/* results state */}
          <div style={{
            "text-align": "center",
            "animation": "fadeIn 0.5s ease"
          }}>
            <div style={{
              "font-size": "8rem",
              "margin-bottom": "2rem",
              "filter": "drop-shadow(0 4px 20px rgba(0,0,0,0.3))"
            }}>
              {(result().connected_peers + result().discovered_peers) >= 2 ? "✅" : "❌"}
            </div>

            <h2 style={{
              "font-size": "2.5rem",
              "margin": "0 0 1rem 0",
              "color": textColor(),
              "font-weight": "900"
            }}>
              {(result().connected_peers + result().discovered_peers) >= 2 ? "connected!" : "failed"}
            </h2>

            <div style={{
              "background": "rgba(0, 0, 0, 0.2)",
              "border-radius": "12px",
              "padding": "1.5rem",
              "margin-bottom": "2rem",
              "backdrop-filter": "blur(10px)"
            }}>
              <div style={{
                "display": "grid",
                "grid-template-columns": "1fr 1fr",
                "gap": "1rem",
                "color": textColor(),
                "font-weight": "600"
              }}>
                <div>
                  <div style={{ "font-size": "2rem" }}>{result().discovered_peers}</div>
                  <div style={{ "font-size": "0.85rem", "opacity": "0.8" }}>discovered</div>
                </div>
                <div>
                  <div style={{ "font-size": "2rem" }}>{result().connected_peers}</div>
                  <div style={{ "font-size": "0.85rem", "opacity": "0.8" }}>connected</div>
                </div>
              </div>
            </div>

            <Show when={result().error}>
              <div style={{
                "background": "rgba(0, 0, 0, 0.3)",
                "border-radius": "8px",
                "padding": "1rem",
                "margin-bottom": "1rem",
                "font-family": "monospace",
                "font-size": "0.85rem",
                "color": textColor(),
                "opacity": "0.9"
              }}>
                {result().error}
              </div>
            </Show>

            <button
              onClick={() => setResult(null)}
              style={{
                "padding": "1rem 2rem",
                "font-size": "1rem",
                "font-weight": "700",
                "border": `2px solid ${textColor()}`,
                "border-radius": "8px",
                "background": "transparent",
                "color": textColor(),
                "cursor": "pointer",
                "transition": "all 0.2s",
                "text-transform": "uppercase",
                "letter-spacing": "0.05em"
              }}
            >
              test another
            </button>
          </div>
        </Show>
      </div>

      <style>{`
        @keyframes fadeIn {
          from { opacity: 0; transform: scale(0.95); }
          to { opacity: 1; transform: scale(1); }
        }

        button:hover:not(:disabled) {
          transform: translateY(-2px);
          box-shadow: 0 8px 20px rgba(0, 0, 0, 0.3);
        }

        input:focus {
          outline: none;
          box-shadow: 0 0 0 3px ${colors.pink};
        }
      `}</style>
    </div>
  );
}

export default App;
