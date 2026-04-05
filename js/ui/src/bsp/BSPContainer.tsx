import {
  createSignal,
  createEffect,
  createMemo,
  onCleanup,
  Show,
  For,
} from "solid-js";
import {
  BlitTerminal,
  BlitSurfaceView,
  createBlitWorkspace,
  createBlitSessions,
  createBlitWorkspaceState,
} from "@blit-sh/solid";
import type { SessionId, TerminalPalette } from "@blit-sh/core";
import type { BSPNode, BSPChild, BSPSplit, BSPLeaf } from "@blit-sh/core/bsp";
import { leafCount, serializeDSL } from "@blit-sh/core/bsp";
import type { BSPAssignments, BSPLayout } from "./layout";
import {
  adjustWeights,
  assignSessionsToPanes,
  buildCandidateOrder,
  enumeratePanes,
  loadAssignmentsFromHash,
  loadFocusedPaneFromHash,
  reconcileAssignments,
  saveActiveLayout,
  isSurfaceAssignment,
  parseSurfaceAssignment,
} from "./layout";
import { ResizeHandle } from "./ResizeHandle";
import type { Theme } from "../theme";
import { themeFor, ui, uiScale } from "../theme";
import { t, tp } from "../i18n";

function resolveLeafFontSize(leaf: BSPLeaf, baseFontSize: number): number {
  const raw = leaf.fontSize;
  if (raw == null) return baseFontSize;
  let resolved: number;
  if (typeof raw === "number") {
    resolved = raw;
  } else if (raw.endsWith("%")) {
    resolved = Math.round((baseFontSize * parseFloat(raw)) / 100);
  } else if (raw.endsWith("pt")) {
    resolved = Math.round((parseFloat(raw) * 4) / 3);
  } else if (raw.endsWith("px")) {
    resolved = parseFloat(raw);
  } else {
    resolved = baseFontSize;
  }
  return Math.max(6, Math.min(72, Math.round(resolved)));
}

function sameAssignments(left: BSPAssignments, right: BSPAssignments): boolean {
  const leftKeys = Object.keys(left.assignments);
  const rightKeys = Object.keys(right.assignments);
  if (leftKeys.length !== rightKeys.length) return false;
  for (const key of leftKeys) {
    if (left.assignments[key] !== right.assignments[key]) return false;
  }
  return true;
}

export function BSPContainer(props: {
  layout: BSPLayout;
  onLayoutChange: (layout: BSPLayout | null) => void;
  connectionId: string;
  palette: TerminalPalette;
  fontFamily: string;
  fontSize: number;

  focusedSessionId: SessionId | null;
  lruSessionIds: readonly SessionId[];
  manageVisibility?: boolean;
  onAssignmentsChange?: (assignments: BSPAssignments) => void;
  onFocusSession: (id: SessionId | null) => void;
  onCreateInPane?: (paneId: string, command?: string) => void;
  /** Called with control functions so the parent can direct pane focus/assignments. */
  onFocusBySession?: (fn: (sessionId: SessionId) => void) => void;
  onFocusPane?: (fn: (paneId: string) => void) => void;
  onMoveSessionToPane?: (
    fn: (sessionId: SessionId, targetPaneId: string) => void,
  ) => void;
  onMoveToPane?: (fn: (value: string, targetPaneId: string) => void) => void;
  onFocusedPaneChange?: (paneId: string | null) => void;
}) {
  const workspace = createBlitWorkspace();
  const workspaceState = createBlitWorkspaceState(workspace);
  const sessions = createBlitSessions(workspace);

  const connection = createMemo(() => {
    const snap = workspaceState();
    return snap.connections.find((c) => c.id === props.connectionId) ?? null;
  });
  const connected = () => connection()?.status === "connected";

  const liveSessions = createMemo(() =>
    sessions().filter(
      (session) =>
        session.connectionId === props.connectionId &&
        session.state !== "closed",
    ),
  );
  const liveSessionIds = createMemo(() =>
    liveSessions().map((session) => session.id),
  );

  const [root, setRoot] = createSignal(props.layout.root);
  const panes = createMemo(() => enumeratePanes(root()));
  const paneIds = createMemo(() => panes().map((pane) => pane.id));

  // Hash assignments store connectionId:ptyId pairs. We resolve them to
  // session IDs once sessions arrive from the server.
  let pendingHash: Record<string, string> | null = loadAssignmentsFromHash();

  const [layoutState, setLayoutState] = createSignal<BSPAssignments>(
    (() => {
      // Don't resolve hash assignments yet — sessions haven't arrived.
      // Start with empty assignments; the effect below will resolve them.
      if (pendingHash) {
        const assignments: Record<string, SessionId | null> = {};
        for (const paneId of paneIds()) {
          assignments[paneId] = null;
        }
        return { assignments };
      }
      const orderedSessionIds = buildCandidateOrder({
        liveSessionIds: liveSessionIds(),
        focusedSessionId: props.focusedSessionId,
        lruSessionIds: props.lruSessionIds,
      });
      return assignSessionsToPanes(panes(), orderedSessionIds);
    })(),
  );

  let lastDsl = props.layout.dsl;
  let lastLayout = props.layout;

  // React to external layout changes.
  createEffect(() => {
    const layout = props.layout;
    if (layout === lastLayout) return;

    const currentPanes = enumeratePanes(root());
    const currentAssignedInPaneOrder = currentPanes
      .map((pane) => layoutState().assignments[pane.id])
      .filter((sessionId): sessionId is SessionId => sessionId != null);
    const orderedSessionIds = buildCandidateOrder({
      liveSessionIds: liveSessionIds(),
      focusedSessionId: props.focusedSessionId,
      currentAssignedInPaneOrder,
      lruSessionIds: props.lruSessionIds,
    });
    const nextRoot = layout.root;
    const nextPanes = enumeratePanes(nextRoot);

    lastLayout = layout;
    lastDsl = layout.dsl;
    setRoot(nextRoot);
    setLayoutState(assignSessionsToPanes(nextPanes, orderedSessionIds));
  });

  const knownSessionIds = createMemo(() =>
    sessions()
      .filter((s) => s.connectionId === props.connectionId)
      .map((s) => s.id),
  );

  // Resolve pending hash assignments (connectionId:ptyId) to session IDs
  // once sessions arrive from the server.
  createEffect(() => {
    if (!pendingHash || liveSessions().length === 0) return;

    const assignments: Record<string, SessionId | null> = {};
    let resolved = 0;
    for (const paneId of paneIds()) {
      const ref = pendingHash[paneId];
      if (!ref) {
        assignments[paneId] = null;
        continue;
      }
      // ref is "connectionId:ptyId" — split on last colon since connectionId might contain colons
      const lastColon = ref.lastIndexOf(":");
      if (lastColon <= 0) {
        assignments[paneId] = null;
        continue;
      }
      const connId = ref.slice(0, lastColon);
      const ptyId = parseInt(ref.slice(lastColon + 1), 10);
      const session = liveSessions().find(
        (s) => s.connectionId === connId && s.ptyId === ptyId,
      );
      assignments[paneId] = session?.id ?? null;
      if (session) resolved++;
    }

    if (resolved > 0) {
      pendingHash = null;
      setLayoutState({ assignments });
    }
  });

  createEffect(() => {
    if (!connected()) return;
    // Skip reconciliation while we still have pending hash assignments to resolve.
    if (pendingHash) return;
    const p = panes();
    const live = liveSessionIds();
    const known = knownSessionIds();
    setLayoutState((previous) => {
      const next = reconcileAssignments({
        panes: p,
        previous,
        liveSessionIds: live,
        knownSessionIds: known,
      });
      return sameAssignments(previous, next) ? previous : next;
    });
  });

  const assignedInPaneOrder = createMemo(() =>
    paneIds()
      .map((paneId) => layoutState().assignments[paneId])
      .filter((v): v is SessionId => v != null && !isSurfaceAssignment(v)),
  );

  // focusedPaneId is the single source of truth for which pane is active.
  const [focusedPaneId, setFocusedPaneId] = createSignal<string | null>(
    (() => {
      const fromHash = loadFocusedPaneFromHash();
      if (fromHash && paneIds().includes(fromHash)) return fromHash;
      if (!props.focusedSessionId) return paneIds()[0] ?? null;
      return (
        paneIds().find(
          (id) => layoutState().assignments[id] === props.focusedSessionId,
        ) ??
        paneIds()[0] ??
        null
      );
    })(),
  );

  // Derive the focused session from the focused pane.
  // Returns null if the pane holds a surface rather than a session.
  const focusedPaneSessionId = createMemo(() => {
    const fpId = focusedPaneId();
    if (!fpId) return null;
    const value = layoutState().assignments[fpId] ?? null;
    return value && !isSurfaceAssignment(value) ? value : null;
  });

  // Keep focusedPaneId valid when panes change.
  createEffect(() => {
    const fpId = focusedPaneId();
    if (fpId != null && !paneIds().includes(fpId)) {
      setFocusedPaneId(paneIds()[0] ?? null);
    }
  });

  // Push our derived session up to Workspace.
  createEffect(() => {
    const fpSessionId = focusedPaneSessionId();
    if (fpSessionId !== props.focusedSessionId) {
      props.onFocusSession(fpSessionId);
    }
  });

  // Allow Workspace to focus a specific session's pane (e.g. from menu).
  function focusBySession(sessionId: SessionId) {
    const paneId = paneIds().find(
      (id) => layoutState().assignments[id] === sessionId,
    );
    if (paneId) setFocusedPaneId(paneId);
  }

  createEffect(() => {
    props.onFocusBySession?.(focusBySession);
  });

  function moveToPane(value: string, targetPaneId: string) {
    setLayoutState((prev) => {
      if (prev.assignments[targetPaneId] === value) return prev;
      return {
        ...prev,
        assignments: {
          ...prev.assignments,
          [targetPaneId]: value,
        },
      };
    });
    setFocusedPaneId(targetPaneId);
  }

  function moveSessionToPane(sessionId: SessionId, targetPaneId: string) {
    moveToPane(sessionId, targetPaneId);
  }

  createEffect(() => {
    props.onMoveSessionToPane?.(moveSessionToPane);
  });
  createEffect(() => {
    props.onMoveToPane?.(moveToPane);
  });

  function focusPane(paneId: string) {
    setFocusedPaneId(paneId);
  }

  // Report focused pane changes.
  createEffect(() => {
    props.onFocusedPaneChange?.(focusedPaneId());
  });

  createEffect(() => {
    props.onFocusPane?.(focusPane);
  });

  // Remember last active tab per tabs container so switching away doesn't reset.
  const tabMemory: Record<string, number> = {};

  // Ctrl-[ / Ctrl-] to cycle panes. Tabs containers automatically
  // switch to show the focused pane.
  createEffect(() => {
    const ids = paneIds();
    const fpId = focusedPaneId();
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
      if (e.key !== "[" && e.key !== "]") return;
      e.preventDefault();
      const idx = fpId ? ids.indexOf(fpId) : -1;
      const delta = e.key === "]" ? 1 : -1;
      const next = (idx + delta + ids.length) % ids.length;
      focusPane(ids[next]);
    };
    window.addEventListener("keydown", handler, true);
    onCleanup(() => window.removeEventListener("keydown", handler, true));
  });

  createEffect(() => {
    props.onAssignmentsChange?.(layoutState());
  });

  createEffect(() => {
    const manageVisibility = props.manageVisibility ?? true;
    if (!manageVisibility) return;
    workspace.setVisibleSessions(assignedInPaneOrder());
  });

  function updateRoot(next: BSPNode) {
    setRoot(next);
    const dsl = serializeDSL(next);
    const updated: BSPLayout = { ...props.layout, root: next, dsl };
    lastLayout = updated;
    lastDsl = dsl;
    saveActiveLayout(updated);
    props.onLayoutChange(updated);
  }

  function handleResize(
    split: BSPSplit,
    indexA: number,
    indexB: number,
    fraction: number,
  ) {
    const updated = adjustWeights(split, indexA, indexB, fraction);
    const replaceNode = (node: BSPNode): BSPNode => {
      if (node === split) return updated;
      if (node.type === "leaf") return node;
      return {
        ...node,
        children: node.children.map((child) => ({
          ...child,
          node: replaceNode(child.node),
        })),
      };
    };
    updateRoot(replaceNode(root()));
  }

  createEffect(() => {
    const fsId = props.focusedSessionId;
    const live = liveSessions();
    const handler = (event: KeyboardEvent) => {
      if (!fsId) return;
      const session = live.find((item) => item.id === fsId);
      if (!session || session.state !== "exited") return;
      if (event.key === "Enter") {
        event.preventDefault();
        workspace.restartSession(fsId);
      } else if (event.key === "Escape") {
        event.preventDefault();
        void workspace.closeSession(fsId);
      }
    };
    window.addEventListener("keydown", handler);
    onCleanup(() => window.removeEventListener("keydown", handler));
  });

  const multiPane = () => leafCount(root()) > 1;

  return (
    <div style={{ width: "100%", height: "100%", display: "flex" }}>
      <BSPPane
        node={root()}
        assignments={layoutState().assignments}
        connectionId={props.connectionId}
        multiPane={multiPane()}
        focusedPaneId={focusedPaneId()}
        onFocusPane={focusPane}
        onCreateInPane={props.onCreateInPane}
        onResize={handleResize}
        palette={props.palette}
        fontFamily={props.fontFamily}
        fontSize={props.fontSize}
        visible={props.manageVisibility ?? true}
        tabMemory={tabMemory}
      />
    </div>
  );
}

function BSPPane(props: {
  node: BSPNode;
  assignments: Record<string, SessionId | null>;
  connectionId: string;
  multiPane: boolean;
  focusedPaneId: string | null;
  onFocusPane: (paneId: string) => void;
  onCreateInPane?: (paneId: string, command?: string) => void;
  onResize: (
    split: BSPSplit,
    indexA: number,
    indexB: number,
    fraction: number,
  ) => void;
  palette: TerminalPalette;
  fontFamily: string;
  fontSize: number;
  visible: boolean;
  tabMemory: Record<string, number>;
  path?: number[];
}) {
  // We need current values for callbacks that close over stale data.
  // In Solid, we read props directly which always has the latest value.

  const path = () => props.path ?? [];

  // --- Leaf ---
  if (props.node.type === "leaf") {
    const paneId = path().length > 0 ? path().join(".") : "0";
    return (
      <LeafPane
        paneId={paneId}
        leaf={props.node}
        sessionId={props.assignments[paneId] ?? null}
        connectionId={props.connectionId}
        multiPane={props.multiPane}
        isFocused={paneId === props.focusedPaneId}
        onFocusPane={() => props.onFocusPane(paneId)}
        onCreateInPane={props.onCreateInPane}
        palette={props.palette}
        fontFamily={props.fontFamily}
        fontSize={props.fontSize}
        visible={props.visible}
      />
    );
  }

  // After the leaf guard above, node must be a split.
  const split = props.node as BSPSplit;

  // --- Tabs ---
  if (split.direction === "tabs") {
    const theme = () => themeFor(props.palette);
    const scale = () => uiScale(props.fontSize);

    const tabKey = path().join(".") || "root";

    const activeTab = () => {
      const focusedPrefix = props.focusedPaneId ?? "";
      let active = -1;
      for (let i = 0; i < split.children.length; i++) {
        const childPrefix = [...path(), i].join(".");
        if (
          focusedPrefix === childPrefix ||
          focusedPrefix.startsWith(childPrefix + ".")
        ) {
          active = i;
          break;
        }
      }
      if (active >= 0) {
        props.tabMemory[tabKey] = active;
        return active;
      }
      return Math.min(props.tabMemory[tabKey] ?? 0, split.children.length - 1);
    };

    const tabLabel = (child: BSPChild, index: number): string => {
      if (child.label) return child.label;
      if (child.node.type === "leaf" && child.node.tag) return child.node.tag;
      return tp("bsp.tab", { index: index + 1 });
    };

    return (
      <div
        style={{
          display: "flex",
          "flex-direction": "column",
          width: "100%",
          height: "100%",
        }}
      >
        <div
          style={{
            display: "flex",
            gap: "1px",
            "flex-shrink": 0,
            "background-color": theme().solidPanelBg,
            "border-bottom": `1px solid ${theme().subtleBorder}`,
            "font-size": `${scale().sm}px`,
          }}
        >
          <For each={split.children}>
            {(child, index) => {
              const childPath = () => [...path(), index()].join(".");
              return (
                <button
                  onClick={() => props.onFocusPane(childPath())}
                  style={{
                    ...ui.btn,
                    flex: 1,
                    "min-width": 0,
                    padding: `${scale().controlY}px ${scale().controlX}px`,
                    "font-size": `${scale().sm}px`,
                    "text-align": "center",
                    overflow: "hidden",
                    "text-overflow": "ellipsis",
                    "white-space": "nowrap",
                    opacity: index() === activeTab() ? 1 : 0.5,
                    "border-bottom":
                      index() === activeTab()
                        ? `1px solid ${theme().accent}`
                        : "1px solid transparent",
                  }}
                >
                  {tabLabel(child, index())}
                </button>
              );
            }}
          </For>
        </div>
        <div
          style={{
            flex: 1,
            overflow: "hidden",
            position: "relative",
            "min-height": 0,
          }}
        >
          <BSPPane
            node={split.children[activeTab()].node}
            assignments={props.assignments}
            connectionId={props.connectionId}
            multiPane={props.multiPane}
            focusedPaneId={props.focusedPaneId}
            onFocusPane={props.onFocusPane}
            onCreateInPane={props.onCreateInPane}
            onResize={props.onResize}
            palette={props.palette}
            fontFamily={props.fontFamily}
            fontSize={props.fontSize}
            visible={props.visible}
            tabMemory={props.tabMemory}
            path={[...path(), activeTab()]}
          />
        </div>
      </div>
    );
  }

  // --- Horizontal / Vertical split ---
  const flexDirection = () =>
    split.direction === "horizontal" ? "row" : "column";

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": flexDirection(),
        width: "100%",
        height: "100%",
      }}
    >
      <For each={split.children}>
        {(child, index) => (
          <>
            <Show when={index() > 0}>
              <ResizeHandle
                direction={split.direction as "horizontal" | "vertical"}
                onDrag={(fraction) =>
                  props.onResize(split, index() - 1, index(), fraction)
                }
              />
            </Show>
            <div
              style={{
                flex: child.weight,
                overflow: "hidden",
                position: "relative",
                "min-width": 0,
                "min-height": 0,
              }}
            >
              <BSPPane
                node={child.node}
                assignments={props.assignments}
                connectionId={props.connectionId}
                multiPane={props.multiPane}
                focusedPaneId={props.focusedPaneId}
                onFocusPane={props.onFocusPane}
                onCreateInPane={props.onCreateInPane}
                onResize={props.onResize}
                palette={props.palette}
                fontFamily={props.fontFamily}
                fontSize={props.fontSize}
                visible={props.visible}
                tabMemory={props.tabMemory}
                path={[...(props.path ?? []), index()]}
              />
            </div>
          </>
        )}
      </For>
    </div>
  );
}

function LeafPane(props: {
  paneId: string;
  leaf: BSPLeaf;
  sessionId: SessionId | null;
  connectionId: string;
  multiPane: boolean;
  isFocused: boolean;
  onFocusPane: () => void;
  onCreateInPane?: (paneId: string, command?: string) => void;
  palette: TerminalPalette;
  fontFamily: string;
  fontSize: number;
  visible: boolean;
}) {
  const theme = () => themeFor(props.palette);
  const scale = () => uiScale(props.fontSize);
  const workspace = createBlitWorkspace();
  const sessions = createBlitSessions(workspace);
  const workspaceState = createBlitWorkspaceState(workspace);

  const surfaceId = () => parseSurfaceAssignment(props.sessionId);
  const isSurface = () => surfaceId() != null;

  const session = () =>
    isSurface()
      ? null
      : (sessions().find((item) => item.id === props.sessionId) ?? null);

  const connection = () => {
    const snap = workspaceState();
    return snap.connections.find((c) => c.id === props.connectionId) ?? null;
  };

  let paneContainer!: HTMLDivElement;
  let autoCreated = false;

  createEffect(() => {
    if (props.sessionId || !props.leaf.command || autoCreated) return;
    if (connection()?.status !== "connected") return;
    autoCreated = true;
    props.onCreateInPane?.(props.paneId, props.leaf.command);
  });

  createEffect(() => {
    // Track these dependencies
    const focused = props.isFocused;
    const _sid = props.sessionId;
    const _vis = props.visible;
    if (focused && paneContainer) {
      // Focus the pane container's focusable child (canvas).
      const focusable = paneContainer.querySelector<HTMLElement>(
        "canvas, [tabindex], input, textarea",
      );
      focusable?.focus();
    }
  });

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        position: "relative",
        border: props.multiPane
          ? props.isFocused
            ? `1px solid ${theme().accent}`
            : "1px solid transparent"
          : "none",
      }}
      onPointerDown={() => props.onFocusPane()}
      onFocusIn={() => props.onFocusPane()}
    >
      <Show when={isSurface()}>
        <div ref={paneContainer} style={{ width: "100%", height: "100%" }}>
          <BlitSurfaceView
            connectionId={props.connectionId}
            surfaceId={surfaceId()!}
            focus={props.isFocused}
            resizable
            style={{ width: "100%", height: "100%" }}
          />
        </div>
      </Show>
      <Show when={!isSurface()}>
        <Show
          when={props.sessionId && session()}
          fallback={
            <Show
              when={connection()?.status === "connected"}
              fallback={
                <div
                  style={{
                    width: "100%",
                    height: "100%",
                    "background-color": `rgb(${props.palette.bg[0]},${props.palette.bg[1]},${props.palette.bg[2]})`,
                  }}
                />
              }
            >
              <EmptyPane
                paneId={props.paneId}
                label={props.leaf.tag || null}
                isFocused={props.isFocused}
                theme={theme()}
                palette={props.palette}
                fontSize={props.fontSize}
                onCreateInPane={props.onCreateInPane}
              />
            </Show>
          }
        >
          <div ref={paneContainer} style={{ width: "100%", height: "100%" }}>
            <BlitTerminal
              sessionId={props.sessionId}
              fontSize={resolveLeafFontSize(props.leaf, props.fontSize)}
              fontFamily={props.fontFamily}
              palette={props.palette}
              style={{ width: "100%", height: "100%" }}
              showCursor={props.isFocused}
            />
          </div>
          <Show when={session()?.state === "exited"}>
            <div
              style={{
                position: "absolute",
                bottom: "8px",
                left: "50%",
                transform: "translateX(-50%)",
                background: theme().solidPanelBg,
                border: `1px solid ${theme().border}`,
                padding: `${scale().controlY}px ${scale().controlX}px`,
                "font-size": `${scale().sm}px`,
                display: "flex",
                "align-items": "center",
                gap: `${scale().gap}px`,
              }}
            >
              <mark
                style={{
                  ...ui.badge,
                  "background-color": "rgba(255,100,100,0.3)",
                }}
              >
                {t("bsp.exited")}
              </mark>
              <Show when={connection()?.supportsRestart}>
                <button
                  onClick={() => workspace.restartSession(props.sessionId!)}
                  style={{ ...ui.btn, "font-size": `${scale().sm}px` }}
                >
                  {t("bsp.restart")} <kbd style={ui.kbd}>Enter</kbd>
                </button>
              </Show>
              <button
                onClick={() => void workspace.closeSession(props.sessionId!)}
                style={{
                  ...ui.btn,
                  "font-size": `${scale().sm}px`,
                  opacity: 0.5,
                }}
              >
                {t("bsp.close")} <kbd style={ui.kbd}>Esc</kbd>
              </button>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
}

function EmptyPane(props: {
  paneId: string;
  label: string | null;
  isFocused: boolean;
  theme: Theme;
  palette: TerminalPalette;
  fontSize: number;
  onCreateInPane?: (paneId: string, command?: string) => void;
}) {
  const [cmd, setCmd] = createSignal("");
  let inputRef!: HTMLInputElement;
  const scale = () => uiScale(props.fontSize);

  createEffect(() => {
    if (props.isFocused) inputRef?.focus();
  });

  return (
    <div
      onClick={() => inputRef?.focus()}
      style={{
        width: "100%",
        height: "100%",
        position: "relative",
        "background-color": `rgb(${props.palette.bg[0]},${props.palette.bg[1]},${props.palette.bg[2]})`,
      }}
    >
      <div
        style={{
          position: "absolute",
          bottom: "8px",
          left: "50%",
          transform: "translateX(-50%)",
          background: props.theme.solidPanelBg,
          border: `1px solid ${props.theme.border}`,
          padding: `${scale().controlY}px ${scale().controlX}px`,
          "font-size": `${scale().sm}px`,
          display: "flex",
          "align-items": "center",
          gap: `${scale().gap}px`,
        }}
      >
        <mark style={ui.badge}>{t("bsp.empty")}</mark>
        <input
          ref={inputRef}
          type="text"
          value={cmd()}
          onInput={(e) => setCmd(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              props.onCreateInPane?.(props.paneId, cmd().trim() || undefined);
            }
          }}
          placeholder={t("bsp.commandPlaceholder")}
          style={{
            ...ui.input,
            flex: "none",
            background: "transparent",
            border: "none",
            color: "inherit",
            padding: 0,
            "font-size": `${scale().sm}px`,
            "font-family": "inherit",
            width: "min(50vw, 220px)",
          }}
        />
      </div>
    </div>
  );
}
