import { createSignal, createEffect, onCleanup, Show } from "solid-js";
import {
  WebSocketTransport,
  WebTransportTransport,
  createShareTransport,
} from "@blit-sh/core";
import type { BlitTransport, BlitWasmModule } from "@blit-sh/core";
import { wsUrl, wtUrl } from "./storage";
import { themeFor } from "./theme";
import { t as i18n } from "./i18n";
import { Workspace } from "./Workspace";
import {
  encryptPassphrase,
  isEncrypted,
  decryptPassphrase,
} from "./passphrase-crypto";

interface BlitConfig {
  gateway?: boolean;
  certHash?: string;
  hub?: string;
  host?: string;
}

function readPassphrase(): string | null {
  const raw = location.hash.slice(1);
  if (!raw) return null;
  const first = raw.split("&")[0];
  if (/^[lpa]=/.test(first)) return null;
  const decoded = decodeURIComponent(first);
  if (isEncrypted(decoded)) return decryptPassphrase(decoded);
  const encrypted = encryptPassphrase(decoded);
  const rest = raw.split("&").slice(1);
  const parts = [encodeURIComponent(encrypted), ...rest].filter(Boolean);
  history.replaceState(null, "", `${location.pathname}#${parts.join("&")}`);
  return decoded;
}

readPassphrase();

async function fetchConfig(): Promise<BlitConfig> {
  const base = location.pathname.endsWith("/")
    ? location.pathname
    : location.pathname + "/";
  const resp = await fetch(base + "config");
  return resp.json();
}

function createTransport(pass: string, config: BlitConfig): BlitTransport {
  if (config.hub) {
    return createShareTransport(config.hub, pass);
  }
  if (typeof WebTransport !== "undefined" && config.certHash) {
    return new WebTransportTransport(wtUrl(), pass, {
      serverCertificateHash: config.certHash,
    });
  }
  return new WebSocketTransport(wsUrl(), pass);
}

export function App(props: { wasm: BlitWasmModule }) {
  const [passphrase, setPassphrase] = createSignal(readPassphrase());
  const [config, setConfig] = createSignal<BlitConfig | null>(null);

  createEffect(() => {
    const onHashChange = () => setPassphrase(readPassphrase());
    window.addEventListener("hashchange", onHashChange);
    onCleanup(() => window.removeEventListener("hashchange", onHashChange));
  });

  fetchConfig().then(setConfig, () => setConfig({ gateway: true }));

  return (
    <Show when={config()}>
      {(cfg) => (
        <Show
          when={passphrase()}
          fallback={<AuthApp wasm={props.wasm} config={cfg()} />}
        >
          {(pass) => (
            <ConnectedApp
              wasm={props.wasm}
              passphrase={pass()}
              config={cfg()}
            />
          )}
        </Show>
      )}
    </Show>
  );
}

function ConnectedApp(props: {
  wasm: BlitWasmModule;
  passphrase: string;
  config: BlitConfig;
}) {
  const transport = createTransport(props.passphrase, props.config);

  return (
    <Workspace
      transport={transport}
      wasm={props.wasm}
      onAuthError={() => {
        history.replaceState(null, "", location.pathname);
        window.location.reload();
      }}
    />
  );
}

function AuthApp(props: { wasm: BlitWasmModule; config: BlitConfig }) {
  const [transport, setTransport] = createSignal<BlitTransport | null>(null);
  const [authError, setAuthError] = createSignal<string | null>(null);
  let passRef!: HTMLInputElement;

  function connect(pass: string) {
    setAuthError(null);
    const t = createTransport(pass, props.config);
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
  }

  return (
    <Show
      when={transport()}
      fallback={
        <AuthScreen
          error={authError()}
          passRef={passRef}
          onSubmit={(pass) => connect(pass)}
        />
      }
    >
      {(t) => (
        <Workspace
          transport={t()}
          wasm={props.wasm}
          onAuthError={() => {
            setTransport(null);
            setAuthError(i18n("auth.failed"));
          }}
        />
      )}
    </Show>
  );
}

function AuthScreen(props: {
  error: string | null;
  passRef: HTMLInputElement;
  onSubmit: (pass: string) => void;
}) {
  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = themeFor(dark);
  let inputRef!: HTMLInputElement;

  return (
    <main
      style={{
        display: "flex",
        "align-items": "center",
        "justify-content": "center",
        height: "100%",
        "background-color": theme.bg,
      }}
    >
      <form
        style={{
          display: "flex",
          "flex-direction": "column",
          gap: "0.5em",
        }}
        onSubmit={(e) => {
          e.preventDefault();
          const v = inputRef?.value;
          if (v) props.onSubmit(v);
        }}
      >
        <input
          ref={inputRef}
          type="password"
          placeholder={i18n("auth.placeholder")}
          autofocus
          style={{
            padding: "0.5em 0.75em",
            "font-size": "1em",
            border: "1px solid #444",
            outline: "none",
            width: "20em",
            "font-family": "inherit",
            "background-color": theme.solidInputBg,
            color: theme.fg,
          }}
        />
        <Show when={props.error}>
          {(err) => (
            <output style={{ color: theme.errorText, "font-size": "0.85em" }}>
              {err()}
            </output>
          )}
        </Show>
      </form>
    </main>
  );
}
