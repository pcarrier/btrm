import { onMount, onCleanup } from "solid-js";
import type { BlitWorkspace, BlitSession, SessionId } from "@blit-sh/core";
import type { Overlay } from "./Workspace";

export interface KeyboardShortcutHandlers {
  workspace: BlitWorkspace;
  /** Current overlay accessor */
  overlay: () => Overlay;
  /** Currently active BSP layout (null = single terminal) */
  activeLayout: () => unknown | null;
  /** Currently focused BSP pane ID */
  bspFocusedPaneId: () => string | null;
  /** Focused session accessor */
  focusedSession: () => BlitSession | null;
  /** All sessions accessor */
  sessions: () => readonly BlitSession[];
  /** Focused session ID accessor */
  focusedSessionId: () => SessionId | null;
  /** Connection supports restart */
  supportsRestart: () => boolean;

  toggleOverlay: (target: Overlay) => void;
  cancelOverlay: () => void;
  toggleDebug: () => void;
  togglePreviewPanel: () => void;
  createAndFocus: () => Promise<void>;
  createInPane: (paneId: string) => Promise<void>;
  handleRestartOrClose: () => void;
}

/**
 * Installs global keyboard shortcuts for the workspace.
 * Must be called inside a Solid component (uses onMount/onCleanup).
 */
export function createKeyboardShortcuts(h: KeyboardShortcutHandlers): void {
  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      if (mod && !e.shiftKey && e.key === "k") {
        e.preventDefault();
        h.toggleOverlay("expose");
        return;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === "?" || e.code === "Slash")) {
        e.preventDefault();
        h.toggleOverlay("help");
        return;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === "~" || e.key === "`")) {
        e.preventDefault();
        h.toggleDebug();
        return;
      }
      if (e.ctrlKey && e.shiftKey && e.key === "B") {
        e.preventDefault();
        h.togglePreviewPanel();
        return;
      }
      if (mod && e.shiftKey && e.key === "Enter") {
        e.preventDefault();
        if (h.activeLayout() && h.bspFocusedPaneId()) {
          void h.createInPane(h.bspFocusedPaneId()!);
        } else {
          void h.createAndFocus();
        }
        return;
      }
      if (
        e.key === "Enter" &&
        !mod &&
        !e.shiftKey &&
        !h.overlay() &&
        !h.activeLayout()
      ) {
        const fid = h.focusedSessionId();
        const focused = fid ? h.sessions().find((s) => s.id === fid) : null;
        if ((focused && focused.state === "exited") || fid == null) {
          e.preventDefault();
          h.handleRestartOrClose();
          return;
        }
      }
      if (mod && e.shiftKey && e.key === "Q") {
        if (h.overlay()) return;
        e.preventDefault();
        const fid = h.focusedSessionId();
        if (fid) void h.workspace.closeSession(fid);
        return;
      }
      if (mod && e.shiftKey && (e.key === "{" || e.key === "}")) {
        e.preventDefault();
        const visible = h
          .sessions()
          .filter((s) => s.state !== "closed")
          .map((s) => s.id);
        const currentId = h.focusedSessionId();
        if (visible.length < 2 || !currentId) return;
        const index = visible.indexOf(currentId);
        const nextId =
          e.key === "}"
            ? visible[(index + 1) % visible.length]
            : visible[(index - 1 + visible.length) % visible.length];
        h.workspace.focusSession(nextId);
        return;
      }
      if (e.key === "Escape") {
        if (h.overlay()) {
          e.preventDefault();
          h.cancelOverlay();
          return;
        }
        const fs = h.focusedSession();
        if (fs?.state === "exited") {
          e.preventDefault();
          void h.workspace.closeSession(fs.id);
        }
      }
    };

    window.addEventListener("keydown", handler, true);
    onCleanup(() => window.removeEventListener("keydown", handler, true));
  });
}
