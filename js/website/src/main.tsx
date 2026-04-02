import { createRoot } from "react-dom/client";
import { lazy, Suspense, useState, useEffect } from "react";
import { Landing } from "./Landing";

const Terminal = lazy(() =>
  import("./Terminal").then((m) => ({ default: m.Terminal })),
);

function usePassphrase(): string | null {
  const [passphrase, setPassphrase] = useState<string | null>(() => {
    const hash = location.hash.slice(1);
    return hash ? decodeURIComponent(hash) : null;
  });

  useEffect(() => {
    if (passphrase) {
      history.replaceState(null, "", location.pathname + location.search);
    }
  }, [passphrase]);

  useEffect(() => {
    const onHash = () => {
      const hash = location.hash.slice(1);
      if (hash) {
        const decoded = decodeURIComponent(hash);
        setPassphrase(decoded);
        history.replaceState(null, "", location.pathname + location.search);
      }
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  return passphrase;
}

function App() {
  const passphrase = usePassphrase();

  if (passphrase) {
    return (
      <Suspense
        fallback={
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              height: "100%",
              background: "#0d1117",
              color: "#c9d1d9",
              fontFamily: "monospace",
            }}
          >
            Loading...
          </div>
        }
      >
        <Terminal passphrase={passphrase} />
      </Suspense>
    );
  }
  return <Landing />;
}

createRoot(document.getElementById("root")!).render(<App />);
