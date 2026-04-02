import {
  createSignal,
  createEffect,
  onCleanup,
  For,
  type Accessor,
} from "solid-js";
import type { BlitSession, SessionId } from "@blit-sh/core";

const TAB_ANIMATION_MS = 200;
const TITLE_DEBOUNCE_MS = 150;

// ---------------------------------------------------------------------------
// Debounced, stable title
// ---------------------------------------------------------------------------

function createStableTitle(
  titleAccessor: Accessor<string | null | undefined>,
  fallback: Accessor<string>,
): Accessor<string> {
  const [stable, setStable] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const title = titleAccessor();
    if (!title) return;
    clearTimeout(timer);
    timer = setTimeout(() => setStable(title), TITLE_DEBOUNCE_MS);
  });

  onCleanup(() => clearTimeout(timer));

  return () => stable() || fallback();
}

// ---------------------------------------------------------------------------
// Animated tab list
// ---------------------------------------------------------------------------

type DisplayTab = {
  session: BlitSession;
  liveIndex: number;
  exiting: boolean;
};

function createAnimatedTabs(sessionsAccessor: Accessor<readonly BlitSession[]>) {
  const [displayTabs, setDisplayTabs] = createSignal<DisplayTab[]>(
    sessionsAccessor().map((s, i) => ({ session: s, liveIndex: i, exiting: false })),
  );
  const [enteringIds, setEnteringIds] = createSignal<Set<string>>(new Set());
  let prevIds = new Set(sessionsAccessor().map((s) => s.id));
  const exitTimers = new Map<string, ReturnType<typeof setTimeout>>();

  createEffect(() => {
    const sessions = sessionsAccessor();
    const currentIds = new Set(sessions.map((s) => s.id));

    const added = new Set<string>();
    for (const id of currentIds) {
      if (!prevIds.has(id)) added.add(id);
    }

    const removed = new Set<string>();
    for (const id of prevIds) {
      if (!currentIds.has(id)) removed.add(id);
    }

    prevIds = currentIds;

    if (added.size === 0 && removed.size === 0) {
      // Only session data changed (title, etc) — update in place.
      setDisplayTabs((prev) => {
        let liveIdx = 0;
        return prev.map((dt) => {
          if (dt.exiting) return dt;
          const updated = sessions.find((s) => s.id === dt.session.id);
          if (!updated) return dt;
          return { session: updated, liveIndex: liveIdx++, exiting: false };
        });
      });
      return;
    }

    // Build new display list
    setDisplayTabs((prev) => {
      const result: DisplayTab[] = prev.map((dt) => {
        if (removed.has(dt.session.id)) {
          return { ...dt, exiting: true };
        }
        const updated = sessions.find((s) => s.id === dt.session.id);
        return updated ? { ...dt, session: updated } : dt;
      });

      for (const s of sessions) {
        if (added.has(s.id)) {
          result.push({ session: s, liveIndex: -1, exiting: false });
        }
      }

      let liveIdx = 0;
      for (const dt of result) {
        if (!dt.exiting) dt.liveIndex = liveIdx++;
      }
      return result;
    });

    if (added.size > 0) {
      setEnteringIds(added);
    }

    for (const id of removed) {
      const existing = exitTimers.get(id);
      if (existing) clearTimeout(existing);

      const timer = setTimeout(() => {
        setDisplayTabs((prev) => prev.filter((dt) => dt.session.id !== id));
        exitTimers.delete(id);
      }, TAB_ANIMATION_MS + 50);
      exitTimers.set(id, timer);
    }
  });

  // Clear entering IDs on next frame
  createEffect(() => {
    const entering = enteringIds();
    if (entering.size === 0) return;
    const raf = requestAnimationFrame(() => setEnteringIds(new Set()));
    onCleanup(() => cancelAnimationFrame(raf));
  });

  onCleanup(() => {
    for (const timer of exitTimers.values()) clearTimeout(timer);
  });

  const gridTemplateColumns = () =>
    displayTabs()
      .map((dt) => {
        if (dt.exiting) return "0fr";
        if (enteringIds().has(dt.session.id)) return "0fr";
        return "1fr";
      })
      .join(" ");

  return { displayTabs, gridTemplateColumns };
}

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

function Tab(props: {
  session: BlitSession;
  index: number;
  isFocused: boolean;
  exiting: boolean;
  onSelect: (id: SessionId) => void;
  onClose: (id: SessionId) => void;
}) {
  const label = createStableTitle(
    () => props.session.title,
    () => `Tab ${props.index + 1}`,
  );

  return (
    <div class="min-w-0 overflow-hidden">
      <div
        role="button"
        tabIndex={0}
        onClick={() => !props.exiting && props.onSelect(props.session.id)}
        onAuxClick={(e: MouseEvent) => {
          if (e.button === 1 && !props.exiting) {
            e.preventDefault();
            props.onClose(props.session.id);
          }
        }}
        class={`group relative flex h-full w-full min-w-0 cursor-pointer items-center whitespace-nowrap border-r border-r-[#222] font-sans text-xs transition-colors ${
          props.isFocused
            ? "bg-[#1a1a1a] font-medium text-neutral-200"
            : "bg-transparent font-normal text-neutral-500 hover:bg-[#151515]"
        }`}
      >
        {/* Close button — left-aligned, visible on hover */}
        <button
          type="button"
          tabIndex={-1}
          onClick={(e: MouseEvent) => {
            e.stopPropagation();
            if (!props.exiting) props.onClose(props.session.id);
          }}
          class="absolute left-1.5 flex h-[18px] w-[18px] shrink-0 cursor-pointer items-center justify-center rounded border-none bg-transparent p-0 text-neutral-500 text-xs leading-none opacity-0 transition-[opacity,background-color,color] duration-100 hover:bg-neutral-700 hover:text-neutral-200 group-hover:opacity-100"
        >
          {"\u00D7"}
        </button>
        {/* Title — centered */}
        <span class="w-full overflow-hidden text-ellipsis px-6 text-center">
          {label()}
        </span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TabBar
// ---------------------------------------------------------------------------

export default function TabBar(props: {
  sessions: readonly BlitSession[];
  focusedSessionId: SessionId | null;
  onSelect: (id: SessionId) => void;
  onClose: (id: SessionId) => void;
  onNew: () => void;
}) {
  const { displayTabs, gridTemplateColumns } = createAnimatedTabs(
    () => props.sessions,
  );

  return (
    <div class="flex h-9 min-h-9 select-none items-stretch overflow-hidden border-b border-[#222] bg-[#111]">
      <div
        class="grid min-w-0 flex-1 items-stretch transition-[grid-template-columns] duration-200 ease-out"
        style={{ "grid-template-columns": gridTemplateColumns() }}
      >
        <For each={displayTabs()}>
          {(dt) => (
            <Tab
              session={dt.session}
              index={dt.liveIndex}
              isFocused={!dt.exiting && dt.session.id === props.focusedSessionId}
              exiting={dt.exiting}
              onSelect={props.onSelect}
              onClose={props.onClose}
            />
          )}
        </For>
      </div>
      <button
        type="button"
        onClick={props.onNew}
        class="flex w-9 shrink-0 cursor-pointer items-center justify-center border-none bg-transparent text-lg text-neutral-500 transition-colors hover:text-neutral-300"
        title="New tab"
      >
        +
      </button>
    </div>
  );
}
