import { useState, useCallback, useRef } from "react";
import { WebSocketTransport, WebTransportTransport } from "@blit-sh/core";
import type { BlitTransport } from "@blit-sh/core";
import type { BlitWasmModule } from "@blit-sh/core";
import { wsUrl, wtUrl, wtCertHash } from "./storage";
import { themeFor } from "./theme";
import { t as i18n } from "./i18n";
import { Workspace } from "./Workspace";
import { createShareTransport } from "@blit-sh/core";
import {
  encryptPassphrase,
  isEncrypted,
  decryptPassphrase,
} from "./passphrase-crypto";

const DEFAULT_HUB = "wss://hub.blit.sh";

let _cachedPassphrase: string | null | undefined;

function initPassphrase(): string | null {
  if (_cachedPassphrase !== undefined) return _cachedPassphrase;
  const raw = location.hash.slice(1);
  if (!raw) {
    _cachedPassphrase = null;
    return null;
  }
  const first = raw.split("&")[0];
  if (/^[lpa]=/.test(first)) {
    _cachedPassphrase = null;
    return null;
  }
  const decoded = decodeURIComponent(first);
  if (isEncrypted(decoded)) {
    _cachedPassphrase = decryptPassphrase(decoded);
    return _cachedPassphrase;
  }
  _cachedPassphrase = decoded;
  const encrypted = encryptPassphrase(decoded);
  const rest = raw.split("&").slice(1);
  const parts = [encodeURIComponent(encrypted), ...rest].filter(Boolean);
  history.replaceState(null, "", `${location.pathname}#${parts.join("&")}`);
  return decoded;
}

initPassphrase();

function createGatewayTransport(pass: string): BlitTransport {
  const certHash = wtCertHash();
  if (typeof WebTransport !== "undefined" && certHash) {
    return new WebTransportTransport(wtUrl(), pass, {
      serverCertificateHash: certHash,
    });
  }
  return new WebSocketTransport(wsUrl(), pass);
}

export function App({ wasm }: { wasm: BlitWasmModule }) {
  const [passphrase] = useState(initPassphrase);

  if (passphrase) {
    return <ConnectedApp wasm={wasm} passphrase={passphrase} />;
  }

  return <AuthApp wasm={wasm} />;
}

function ConnectedApp({
  wasm,
  passphrase,
}: {
  wasm: BlitWasmModule;
  passphrase: string;
}) {
  const [transport] = useState<BlitTransport>(() => {
    const hubInjected = (window as unknown as { __blitHub?: string }).__blitHub;
    if (hubInjected) {
      return createShareTransport(hubInjected, passphrase);
    }
    const certHash = wtCertHash();
    if (certHash) {
      return createGatewayTransport(passphrase);
    }
    return createShareTransport(DEFAULT_HUB, passphrase);
  });

  return (
    <Workspace
      transport={transport}
      wasm={wasm}
      onAuthError={() => {
        history.replaceState(null, "", location.pathname);
        window.location.reload();
      }}
    />
  );
}

function AuthApp({ wasm }: { wasm: BlitWasmModule }) {
  const [transport, setTransport] = useState<BlitTransport | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);
  const passRef = useRef<HTMLInputElement | null>(null);

  const connect = useCallback((pass: string) => {
    setAuthError(null);
    const t = createGatewayTransport(pass);
    const onStatus = (status: string) => {
      if (status === "connected") {
        const encrypted = encryptPassphrase(pass);
        history.replaceState(
          null,
          "",
          `${location.pathname}#${encodeURIComponent(encrypted)}`,
        );
        t.removeEventListener("statuschange", onStatus);
      } else if (status === "error") {
        setAuthError(t.lastError ?? i18n("auth.failed"));
        t.removeEventListener("statuschange", onStatus);
        setTransport(null);
      }
    };
    t.addEventListener("statuschange", onStatus);
    setTransport(t);
  }, []);

  if (!transport) {
    return (
      <AuthScreen
        error={authError}
        passRef={passRef}
        onSubmit={(pass) => connect(pass)}
      />
    );
  }

  return (
    <Workspace
      transport={transport}
      wasm={wasm}
      onAuthError={() => {
        setTransport(null);
        setAuthError(i18n("auth.failed"));
      }}
    />
  );
}

function AuthScreen({
  error,
  passRef,
  onSubmit,
}: {
  error: string | null;
  passRef: React.RefObject<HTMLInputElement | null>;
  onSubmit: (pass: string) => void;
}) {
  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = themeFor(dark);
  return (
    <main
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        height: "100%",
        backgroundColor: theme.bg,
      }}
    >
      <form
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5em",
        }}
        onSubmit={(e) => {
          e.preventDefault();
          const v = passRef.current?.value;
          if (v) onSubmit(v);
        }}
      >
        <input
          ref={passRef}
          type="password"
          placeholder={i18n("auth.placeholder")}
          autoFocus
          style={{
            padding: "0.5em 0.75em",
            fontSize: "1em",
            border: "1px solid #444",
            outline: "none",
            width: "20em",
            fontFamily: "inherit",
            backgroundColor: theme.solidInputBg,
            color: theme.fg,
          }}
        />
        {error && (
          <output style={{ color: theme.errorText, fontSize: "0.85em" }}>
            {error}
          </output>
        )}
      </form>
    </main>
  );
}
