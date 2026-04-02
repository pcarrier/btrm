import { createSignal, onCleanup } from "solid-js";
import type {
  BlitWorkspaceSnapshot,
  BlitSession,
  BlitConnectionSnapshot,
  SessionId,
  ConnectionId,
} from "@blit-sh/core";
import { useBlitContext } from "./BlitContext";

export function useBlitWorkspaceState(): () => BlitWorkspaceSnapshot {
  const ctx = useBlitContext();
  const [snap, setSnap] = createSignal(ctx.workspace.getSnapshot());
  const unsub = ctx.workspace.subscribe(() => setSnap(ctx.workspace.getSnapshot()));
  onCleanup(unsub);
  return snap;
}

export function useBlitSession(sessionId: SessionId | null | undefined): () => BlitSession | undefined {
  const state = useBlitWorkspaceState();
  return () => {
    if (sessionId == null) return undefined;
    return state().sessions.find((s) => s.id === sessionId);
  };
}

export function useBlitConnection(connectionId: ConnectionId | null | undefined): () => BlitConnectionSnapshot | undefined {
  const state = useBlitWorkspaceState();
  return () => {
    if (connectionId == null) return undefined;
    return state().connections.find((c) => c.id === connectionId);
  };
}
