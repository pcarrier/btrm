import {
  createSignal,
  createEffect,
  createMemo,
  onMount,
  onCleanup,
  Show,
} from "solid-js";
import {
  BlitTerminal,
  BlitWorkspaceProvider,
  createBlitWorkspace,
  createBlitWorkspaceState,
  createBlitSessions,
} from "@blit-sh/solid";
import { BlitWorkspace, PALETTES } from "@blit-sh/core";
import type {
  BlitSession,
  BlitWasmModule,
  SessionId,
} from "@blit-sh/core";
import { initWasm } from "../../lib/wasm";
import {
  isRawPassphrase,
  encryptPassphrase,
  decryptPassphrase,
} from "../../lib/passphrase-crypto";
import { createDebugLog, type DebugLog } from "./DebugPanel";
import TabBar from "./TabBar";
import StatusOverlay from "./StatusOverlay";
import ShareButton from "./ShareButton";
import ShortcutsPanel from "./ShortcutsPanel";
import DebugPanel from "./DebugPanel";

const HUB_URL = "wss://hub.blit.sh";
const CONNECTION_ID = "main";
const FONT_FAMILY = "Fira Code, ui-monospace, monospace";
const FONT_SIZE = 14;

const GITHUB_DARK = PALETTES.find((p) => p.id === "github-dark")!;
const GITHUB_LIGHT = PALETTES.find((p) => p.id === "github-light")!;

// ---------------------------------------------------------------------------
// Passphrase resolution
// ---------------------------------------------------------------------------

type PassphraseResult =
  | { ok: true; passphrase: string }
  | { ok: false; error: string };

function resolvePassphrase(): PassphraseResult {
  const raw = decodeURIComponent(location.hash.slice(1));
  if (!raw) {
    return { ok: false, error: "No session specified." };
  }
  if (isRawPassphrase(raw)) {
    // Raw UUID — encrypt and replace URL
    const encrypted = encryptPassphrase(raw);
    history.replaceState(null, "", `/s#${encodeURIComponent(encrypted)}`);
    return { ok: true, passphrase: raw };
  }
  // Try to decrypt
  const decrypted = decryptPassphrase(raw);
  if (decrypted === null) {
    return {
      ok: false,
      error: "Cannot decrypt session link. This link was created on a different device.",
    };
  }
  return { ok: true, passphrase: decrypted };
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

export default function TerminalDemo() {
  const [result, setResult] = createSignal<PassphraseResult | null>(null);
  const [wasm, setWasm] = createSignal<BlitWasmModule | null>(null);
  const [wasmError, setWasmError] = createSignal<string | null>(null);

  onMount(() => {
    const r = resolvePassphrase();
    setResult(r);
    if (r.ok) {
      initWasm()
        .then(setWasm)
        .catch((e) => setWasmError(String(e)));
    }
  });

  return (
    <>
      <Show when={result()?.ok === false}>
        <StatusOverlay
          status={(result() as { ok: false; error: string }).error}
          isError
        />
      </Show>
      <Show when={wasmError()}>
        <StatusOverlay status={`Failed to load: ${wasmError()}`} isError />
      </Show>
      <Show when={!result()}>
        <StatusOverlay status="Loading..." />
      </Show>
      <Show when={result()?.ok && !wasm() && !wasmError()}>
        <StatusOverlay status="Loading WASM..." />
      </Show>
      <Show when={result()?.ok && wasm()}>
        <TerminalInner
          wasm={wasm()!}
          passphrase={(result() as { ok: true; passphrase: string }).passphrase}
        />
      </Show>
    </>
  );
}

// ---------------------------------------------------------------------------
// TerminalInner: workspace setup + tab shell
// ---------------------------------------------------------------------------

function TerminalInner(props: {
  wasm: BlitWasmModule;
  passphrase: string;
}) {
  const debugLog = createDebugLog();
  const [debugOpen, setDebugOpen] = createSignal(false);

  // Theme
  const COOKIE_MAX_AGE = 60 * 60 * 24 * 400;
  const [dark, setDark] = createSignal(true);
  onMount(() => {
    setDark(document.documentElement.getAttribute("data-theme") !== "light");
  });
  const toggleTheme = () => {
    const next = !dark();
    setDark(next);
    const name = next ? "dark" : "light";
    localStorage.setItem("blit-theme", name);
    document.cookie = `blit-theme=${name};path=/;max-age=${COOKIE_MAX_AGE};SameSite=Lax`;
    document.documentElement.setAttribute("data-theme", name);
  };

  // Keyboard shortcut for debug panel
  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "d") {
        e.preventDefault();
        setDebugOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", handler);
    onCleanup(() => window.removeEventListener("keydown", handler));
  });

  const workspace = new BlitWorkspace({
    wasm: props.wasm,
    connections: [
      {
        id: CONNECTION_ID,
        transport: {
          type: "share",
          hubUrl: HUB_URL,
          passphrase: props.passphrase,
          debug: debugLog,
        },
      },
    ],
  });

  onCleanup(() => workspace.dispose());

  const palette = () => (dark() ? GITHUB_DARK : GITHUB_LIGHT);

  return (
    <BlitWorkspaceProvider
      workspace={workspace}
      palette={palette()}
      fontFamily={FONT_FAMILY}
      fontSize={FONT_SIZE}
    >
      <TabShell
        workspace={workspace}
        palette={palette}
        dark={dark}
        passphrase={props.passphrase}
        onToggleTheme={toggleTheme}
      />
      <Show when={debugOpen()}>
        <DebugPanel log={debugLog} onClose={() => setDebugOpen(false)} />
      </Show>
    </BlitWorkspaceProvider>
  );
}

// ---------------------------------------------------------------------------
// TabShell: manages sessions, tab bar, terminal rendering
// ---------------------------------------------------------------------------

function TabShell(props: {
  workspace: BlitWorkspace;
  palette: () => typeof GITHUB_DARK;
  dark: () => boolean;
  passphrase: string;
  onToggleTheme: () => void;
}) {
  const workspace = props.workspace;
  const state = createBlitWorkspaceState(workspace);
  const sessions = createBlitSessions(workspace);

  const [showShortcuts, setShowShortcuts] = createSignal(false);

  const visibleSessions = createMemo(() =>
    sessions().filter((s) => s.state !== "closed"),
  );
  const focusedId = createMemo(() => state().focusedSessionId);

  // Track previous focused for fallback
  let prevFocused: { id: SessionId; index: number } | null = null;
  createEffect(() => {
    const fid = focusedId();
    if (fid) {
      const idx = visibleSessions().findIndex((s) => s.id === fid);
      if (idx >= 0) prevFocused = { id: fid, index: idx };
    }
  });

  // Auto-create session when connected + no visible sessions
  let creating = false;
  createEffect(() => {
    const conn = state().connections[0];
    if (
      conn?.status === "connected" &&
      conn?.ready &&
      visibleSessions().length === 0 &&
      !creating
    ) {
      creating = true;
      workspace
        .createSession({
          connectionId: CONNECTION_ID,
          rows: 24,
          cols: 80,
        })
        .then((s) => workspace.focusSession(s.id))
        .finally(() => { creating = false; });
    } else if (visibleSessions().length > 0 && !focusedId()) {
      const idx = prevFocused
        ? Math.min(prevFocused.index, visibleSessions().length - 1)
        : 0;
      workspace.focusSession(visibleSessions()[idx].id);
    }
  });

  // Keep visible sessions in sync
  createEffect(() => {
    const desired = new Set<SessionId>();
    const vs = visibleSessions();
    for (const s of vs) desired.add(s.id);
    workspace.setVisibleSessions(desired);
  });

  const focusedSession = createMemo(() =>
    sessions().find((s) => s.id === focusedId()),
  );
  const focusedExited = createMemo(
    () => focusedSession()?.state === "exited",
  );

  // Handle Enter/Esc on exited sessions
  createEffect(() => {
    const fid = focusedId();
    if (!focusedExited() || !fid) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        workspace.restartSession(fid);
      } else if (e.key === "Escape") {
        e.preventDefault();
        workspace.closeSession(fid);
      }
    };
    window.addEventListener("keydown", handler);
    onCleanup(() => window.removeEventListener("keydown", handler));
  });

  // If focused session vanished, focus the last visible
  createEffect(() => {
    const fid = focusedId();
    const vs = visibleSessions();
    if (fid && vs.every((s) => s.id !== fid)) {
      const next = vs[vs.length - 1];
      workspace.focusSession(next?.id ?? null);
    }
  });

  // Keyboard shortcuts
  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && e.key === "Enter") {
        e.preventDefault();
        workspace
          .createSession({ connectionId: CONNECTION_ID, rows: 24, cols: 80 })
          .then((s) => workspace.focusSession(s.id))
          .catch(() => {});
      } else if (
        e.ctrlKey &&
        e.shiftKey &&
        (e.key === "?" || e.code === "Slash")
      ) {
        e.preventDefault();
        setShowShortcuts((v) => !v);
      } else if (mod && !e.shiftKey && (e.key === "[" || e.key === "]")) {
        e.preventDefault();
        const vs = visibleSessions();
        const fid = focusedId();
        if (vs.length < 2 || !fid) return;
        const idx = vs.findIndex((s) => s.id === fid);
        const next =
          e.key === "]"
            ? vs[(idx + 1) % vs.length]
            : vs[(idx - 1 + vs.length) % vs.length];
        workspace.focusSession(next.id);
      }
    };
    window.addEventListener("keydown", handler, true);
    onCleanup(() => window.removeEventListener("keydown", handler, true));
  });

  // Connection status text
  const statusText = createMemo(() => {
    const conn = state().connections[0];
    if (!conn) return "Connecting...";
    if (conn.status === "connected") {
      return visibleSessions().length === 0
        ? "Connected \u2014 waiting for terminal sessions..."
        : null;
    }
    if (conn.status === "connecting") return "Connecting \u2014 waiting for blit share...";
    if (conn.status === "error") return `Error: ${conn.error ?? "unknown"}`;
    if (conn.status === "disconnected") return "Disconnected";
    return "Connecting...";
  });

  const handleSelectTab = (id: SessionId) => workspace.focusSession(id);
  const handleCloseTab = (id: SessionId) => workspace.closeSession(id);
  const handleNewTab = () => {
    const fid = focusedId();
    workspace
      .createSession({
        connectionId: CONNECTION_ID,
        rows: 24,
        cols: 80,
        ...(fid ? { cwdFromSessionId: fid } : {}),
      })
      .then((s) => workspace.focusSession(s.id))
      .catch(() => {});
  };

  return (
    <div class="fixed inset-0 z-50 flex flex-col bg-[#0a0a0a]">
      <Show when={visibleSessions().length > 0}>
        <div class="flex items-stretch shrink-0">
          <div class="flex-1 min-w-0">
            <TabBar
              sessions={visibleSessions()}
              focusedSessionId={focusedId()}
              onSelect={handleSelectTab}
              onClose={handleCloseTab}
              onNew={handleNewTab}
            />
          </div>
          <ShareButton passphrase={props.passphrase} />
          <button
            onClick={() => setShowShortcuts(true)}
            class="bg-transparent border-none text-neutral-500 cursor-pointer px-2.5 text-sm font-mono font-bold shrink-0 transition-colors hover:text-neutral-300"
            title="Keyboard shortcuts"
          >
            ?
          </button>
          <button
            onClick={props.onToggleTheme}
            class="flex items-center justify-center bg-transparent border-none text-neutral-500 cursor-pointer px-2.5 shrink-0 transition-colors hover:text-neutral-300"
            aria-label={`Switch to ${props.dark() ? "light" : "dark"} mode`}
          >
            {props.dark() ? (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <circle
                  cx="8"
                  cy="8"
                  r="3.5"
                  stroke="currentColor"
                  stroke-width="1.5"
                />
                <path
                  d="M8 1v2M8 13v2M1 8h2M13 8h2M3.05 3.05l1.41 1.41M11.54 11.54l1.41 1.41M3.05 12.95l1.41-1.41M11.54 4.46l1.41-1.41"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                />
              </svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path
                  d="M14 9.2A6 6 0 0 1 6.8 2 6 6 0 1 0 14 9.2Z"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linejoin="round"
                />
              </svg>
            )}
          </button>
        </div>
      </Show>

      <div class="flex-1 overflow-hidden relative p-2 pb-1">
        <Show when={statusText()}>
          <StatusOverlay status={statusText()!} />
        </Show>
        <Show when={focusedId()}>
          <BlitTerminal
            sessionId={focusedId()!}
            fontFamily={FONT_FAMILY}
            fontSize={FONT_SIZE}
            palette={props.palette()}
            style={{ width: "100%", height: "100%" }}
          />
        </Show>
        <Show when={focusedExited()}>
          <div class="absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-3 z-[2] px-4 py-2 bg-[rgba(10,10,10,0.9)] backdrop-blur-sm border border-[#333] rounded-xl font-mono text-[13px] text-neutral-400 whitespace-nowrap">
            <span>Exited</span>
            <span
              role="button"
              tabIndex={0}
              class="cursor-pointer px-3 py-1 rounded-md border border-white/15 bg-white/5 transition-colors hover:bg-white/10"
              onClick={() => workspace.restartSession(focusedId()!)}
            >
              Enter &mdash; reopen
            </span>
            <span
              role="button"
              tabIndex={0}
              class="cursor-pointer px-3 py-1 rounded-md border border-white/15 bg-white/5 transition-colors hover:bg-white/10"
              onClick={() => workspace.closeSession(focusedId()!)}
            >
              Esc &mdash; close
            </span>
          </div>
        </Show>
      </div>
      <Show when={showShortcuts()}>
        <ShortcutsPanel onClose={() => setShowShortcuts(false)} />
      </Show>
    </div>
  );
}
