import { useCallback, useRef, useSyncExternalStore } from 'react';
import type { BlitTransport, BlitSession, ConnectionStatus } from '../types';
import { useBlitConnection } from './useBlitConnection';

export interface UseBlitSessionsOptions {
  /** Automatically create a PTY if the initial list is empty. Default: false. */
  autoCreateIfEmpty?: boolean;
  /** Tag to assign to auto-created PTYs (requires autoCreateIfEmpty). */
  autoCreateTag?: string;
  /** Command to run in auto-created PTYs (requires autoCreateIfEmpty). */
  autoCreateCommand?: string;
  /** Returns the initial terminal size for auto-created PTYs. */
  getInitialSize?: () => { rows: number; cols: number };
  /**
   * Optional accessor for the WASM Terminal instance backing a PTY.
   * When provided, the hook reads `terminal.title()` after each S2C_UPDATE
   * frame and updates `session.title` — matching the upstream web client
   * behavior for OSC 0/2 title sequences.
   */
  getTerminal?: (ptyId: number) => { title(): string } | null;
  /** Called when a new session appears (server-initiated or from createPty). */
  onSessionCreated?: (session: BlitSession) => void;
  /** Called when a session is closed (server-initiated or from closePty). */
  onSessionClosed?: (session: BlitSession) => void;
  /** Called when the transport disconnects. */
  onDisconnect?: () => void;
  /** Called when the transport reconnects (status becomes 'connected'). */
  onReconnect?: () => void;
}

export function useBlitSessions(
  transport: BlitTransport,
  options?: UseBlitSessionsOptions,
) {
  const autoCreate = options?.autoCreateIfEmpty ?? false;
  const autoTag = options?.autoCreateTag;
  const autoCommand = options?.autoCreateCommand;
  const getSize = options?.getInitialSize ?? (() => ({ rows: 24, cols: 80 }));
  const getTerminalRef = useRef(options?.getTerminal);
  getTerminalRef.current = options?.getTerminal;
  const lifecycleRef = useRef(options);
  lifecycleRef.current = options;

  const sessionsRef = useRef<readonly BlitSession[]>([]);
  const listenersRef = useRef(new Set<() => void>());
  const readyRef = useRef(false);
  const readyListenersRef = useRef(new Set<() => void>());
  const focusedPtyIdRef = useRef<number | null>(null);
  const focusedListenersRef = useRef(new Set<() => void>());
  const pendingCreatesRef = useRef<Array<(ptyId: number) => void>>([]);
  const pendingClosesRef = useRef<Map<number, Array<() => void>>>(new Map());
  const wasConnectedRef = useRef(transport.status === 'connected');

  const notify = useCallback(() => {
    for (const l of listenersRef.current) l();
  }, []);

  const notifyReady = useCallback(() => {
    for (const l of readyListenersRef.current) l();
  }, []);

  const notifyFocused = useCallback(() => {
    for (const l of focusedListenersRef.current) l();
  }, []);

  const setFocused = useCallback(
    (ptyId: number | null) => {
      if (focusedPtyIdRef.current !== ptyId) {
        focusedPtyIdRef.current = ptyId;
        notifyFocused();
      }
    },
    [notifyFocused],
  );

  const upsert = useCallback(
    (ptyId: number, tag: string, patch: Partial<BlitSession>) => {
      const prev = sessionsRef.current;
      const idx = prev.findIndex((s) => s.ptyId === ptyId);
      if (idx >= 0) {
        const updated = { ...prev[idx], tag, ...patch };
        const next = [...prev];
        next[idx] = updated;
        sessionsRef.current = next;
      } else {
        sessionsRef.current = [
          ...prev,
          { ptyId, tag, title: null, state: 'active' as const, ...patch },
        ];
      }
      notify();
    },
    [notify],
  );

  // --- Connection callbacks ---

  const onCreated = useCallback(
    (ptyId: number, tag: string) => {
      upsert(ptyId, tag, { state: 'active' });
      const resolve = pendingCreatesRef.current.shift();
      if (resolve) resolve(ptyId);
      const session = sessionsRef.current.find((s) => s.ptyId === ptyId);
      if (session) lifecycleRef.current?.onSessionCreated?.(session);
    },
    [upsert],
  );

  const onClosed = useCallback(
    (closedId: number) => {
      const prev = sessionsRef.current;
      const idx = prev.findIndex((s) => s.ptyId === closedId);
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = { ...next[idx], state: 'closed' };
        sessionsRef.current = next;
        notify();
        lifecycleRef.current?.onSessionClosed?.(next[idx]);
        const resolvers = pendingClosesRef.current.get(closedId);
        if (resolvers) {
          pendingClosesRef.current.delete(closedId);
          for (const r of resolvers) r();
        }
      }
      if (focusedPtyIdRef.current === closedId) {
        const nextActive = sessionsRef.current.find((s) => s.state === 'active');
        setFocused(nextActive?.ptyId ?? null);
      }
    },
    [notify, setFocused],
  );

  const onList = useCallback(
    (entries: { ptyId: number; tag: string }[]) => {
      const ids = new Set(entries.map((e) => e.ptyId));
      let next = sessionsRef.current
        .map((s) => (ids.has(s.ptyId) ? s : { ...s, state: 'closed' as const }));
      for (const { ptyId, tag } of entries) {
        if (!next.some((s) => s.ptyId === ptyId)) {
          next = [...next, { ptyId, tag, title: null, state: 'active' as const }];
        }
      }
      sessionsRef.current = next;

      if (!readyRef.current) {
        readyRef.current = true;
        notifyReady();
      }
      notify();

      if (focusedPtyIdRef.current !== null && !ids.has(focusedPtyIdRef.current)) {
        const nextActive = next.find((s) => s.state === 'active');
        setFocused(nextActive?.ptyId ?? null);
      } else if (focusedPtyIdRef.current === null && entries.length > 0) {
        setFocused(entries[0].ptyId);
      }

      if (autoCreate && entries.length === 0) {
        const { rows, cols } = getSize();
        sendCreateRef.current(rows, cols, {
          tag: autoTag,
          command: autoCommand,
        });
      }
    },
    [notify, notifyReady, setFocused, autoCreate, autoTag, autoCommand, getSize],
  );

  const onTitle = useCallback(
    (ptyId: number, title: string) => {
      const prev = sessionsRef.current;
      const idx = prev.findIndex((s) => s.ptyId === ptyId);
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = { ...next[idx], title };
        sessionsRef.current = next;
        notify();
      }
    },
    [notify],
  );

  const onUpdate = useCallback(
    (ptyId: number, _payload: Uint8Array) => {
      const getTerminal = getTerminalRef.current;
      if (!getTerminal) return;
      const terminal = getTerminal(ptyId);
      if (!terminal) return;
      const title = terminal.title();
      const prev = sessionsRef.current;
      const idx = prev.findIndex((s) => s.ptyId === ptyId);
      if (idx >= 0 && prev[idx].title !== title) {
        const next = [...prev];
        next[idx] = { ...next[idx], title };
        sessionsRef.current = next;
        notify();
      }
    },
    [notify],
  );

  const onStatusChange = useCallback(
    (newStatus: ConnectionStatus) => {
      if (newStatus === 'disconnected' || newStatus === 'error') {
        if (wasConnectedRef.current) {
          wasConnectedRef.current = false;
          lifecycleRef.current?.onDisconnect?.();
        }
      } else if (newStatus === 'connected') {
        if (!wasConnectedRef.current) {
          wasConnectedRef.current = true;
          lifecycleRef.current?.onReconnect?.();
        }
      }
    },
    [],
  );

  const sendCreateRef = useRef<(rows: number, cols: number, options?: { tag?: string; command?: string }) => void>(() => {});

  const { status, sendCreate, sendFocus, sendClose } = useBlitConnection(
    transport,
    { onCreated, onClosed, onList, onTitle, onUpdate, onStatusChange },
  );
  sendCreateRef.current = sendCreate;

  // --- Public API ---

  const sessions = useSyncExternalStore(
    useCallback(
      (l: () => void) => {
        listenersRef.current.add(l);
        return () => listenersRef.current.delete(l);
      },
      [],
    ),
    useCallback(() => sessionsRef.current, []),
    useCallback(() => sessionsRef.current, []),
  );

  const ready = useSyncExternalStore(
    useCallback(
      (l: () => void) => {
        readyListenersRef.current.add(l);
        return () => readyListenersRef.current.delete(l);
      },
      [],
    ),
    useCallback(() => readyRef.current, []),
    useCallback(() => readyRef.current, []),
  );

  const focusedPtyId = useSyncExternalStore(
    useCallback(
      (l: () => void) => {
        focusedListenersRef.current.add(l);
        return () => focusedListenersRef.current.delete(l);
      },
      [],
    ),
    useCallback(() => focusedPtyIdRef.current, []),
    useCallback(() => focusedPtyIdRef.current, []),
  );

  const createPty = useCallback(
    (opts?: { rows?: number; cols?: number; command?: string; tag?: string }): Promise<number> => {
      const { rows, cols } = getSize();
      return new Promise<number>((resolve) => {
        pendingCreatesRef.current.push(resolve);
        sendCreate(opts?.rows ?? rows, opts?.cols ?? cols, {
          tag: opts?.tag,
          command: opts?.command,
        });
      });
    },
    [sendCreate, getSize],
  );

  const focusPty = useCallback(
    (ptyId: number) => {
      setFocused(ptyId);
      sendFocus(ptyId);
    },
    [sendFocus, setFocused],
  );

  const closePty = useCallback(
    (ptyId: number): Promise<void> => {
      return new Promise<void>((resolve) => {
        const existing = pendingClosesRef.current.get(ptyId);
        if (existing) {
          existing.push(resolve);
        } else {
          pendingClosesRef.current.set(ptyId, [resolve]);
        }
        sendClose(ptyId);
      });
    },
    [sendClose],
  );

  return { ready, sessions, status, focusedPtyId, createPty, focusPty, closePty } as const;
}
