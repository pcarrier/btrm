import {
  useState,
  useCallback,
  useEffect,
  useRef,
} from "react";
import { PALETTES } from "blit-react";
import type { TerminalPalette } from "blit-react";
import { styles } from "./styles";

export function PaletteOverlay({
  current,
  onSelect,
  onPreview,
  onClose,
  dark,
}: {
  current: TerminalPalette;
  onSelect: (p: TerminalPalette) => void;
  onPreview: (p: TerminalPalette) => void;
  onClose: () => void;
  dark: boolean;
}) {
  const originalRef = useRef(current);
  const initialIdx = PALETTES.findIndex((p) => p.id === current.id);
  const [selectedIdx, setSelectedIdx] = useState(initialIdx >= 0 ? initialIdx : 0);
  const listRef = useRef<HTMLMenuElement>(null);

  const dismiss = useCallback(() => {
    onPreview(originalRef.current);
    onClose();
  }, [onPreview, onClose]);

  const preview = useCallback((idx: number) => {
    setSelectedIdx(idx);
    onPreview(PALETTES[idx]);
  }, [onPreview]);

  useEffect(() => {
    listRef.current?.focus();
  }, []);

  useEffect(() => {
    const el = listRef.current?.children[selectedIdx + 1] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIdx]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          preview((selectedIdx + 1) % PALETTES.length);
          break;
        case "ArrowUp":
          e.preventDefault();
          preview((selectedIdx - 1 + PALETTES.length) % PALETTES.length);
          break;
        case "Enter":
          e.preventDefault();
          onSelect(PALETTES[selectedIdx]);
          break;
        case "Escape":
          e.preventDefault();
          dismiss();
          break;
      }
    },
    [selectedIdx, onSelect, preview, dismiss],
  );

  return (
    <div
      open
      style={styles.overlay}
      onClick={dismiss}
    >
      <menu
        ref={listRef}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        style={{
          ...styles.paletteBox,
          backgroundColor: dark ? "#1e1e1e" : "#f5f5f5",
          color: dark ? "#e0e0e0" : "#333",
          outline: "none",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <li style={{ fontWeight: 600, marginBottom: 8, listStyle: "none" }}>Palette</li>
        {PALETTES.map((p, i) => (
          <li key={p.id} style={{ listStyle: "none" }}>
            <button
              onClick={() => onSelect(p)}
              onMouseEnter={() => preview(i)}
              style={{
                ...styles.paletteItem,
                backgroundColor:
                  i === selectedIdx
                    ? dark
                      ? "rgba(255,255,255,0.1)"
                      : "rgba(0,0,0,0.08)"
                    : "transparent",
              }}
            >
              <span style={styles.paletteSwatches}>
                <span
                  style={{
                    ...styles.swatch,
                    backgroundColor: `rgb(${p.bg[0]},${p.bg[1]},${p.bg[2]})`,
                    border: "1px solid rgba(128,128,128,0.3)",
                  }}
                />
                <span
                  style={{
                    ...styles.swatch,
                    backgroundColor: `rgb(${p.fg[0]},${p.fg[1]},${p.fg[2]})`,
                  }}
                />
                {p.ansi.slice(0, 8).map((c, j) => (
                  <span
                    key={j}
                    style={{
                      ...styles.swatch,
                      backgroundColor: `rgb(${c[0]},${c[1]},${c[2]})`,
                    }}
                  />
                ))}
              </span>
              <span style={{ fontSize: 13 }}>{p.name}</span>
            </button>
          </li>
        ))}
      </menu>
    </div>
  );
}
