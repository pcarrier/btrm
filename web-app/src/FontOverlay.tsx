import {
  useState,
  useEffect,
  useRef,
  useCallback,
} from "react";
import { DEFAULT_FONT } from "blit-react";
import { styles } from "./styles";

export function FontOverlay({
  currentFamily,
  currentSize,
  serverFonts,
  onSelect,
  onPreview,
  onClose,
  dark,
}: {
  currentFamily: string;
  currentSize: number;
  serverFonts: string[];
  onSelect: (font: string, size: number) => void;
  onPreview: (font: string, size: number) => void;
  onClose: () => void;
  dark: boolean;
}) {
  const [query, setQuery] = useState("");
  const [size, setSize] = useState(currentSize);
  const [selectedIdx, setSelectedIdx] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const originalRef = useRef({ family: currentFamily, size: currentSize });

  const dismiss = useCallback(() => {
    onPreview(originalRef.current.family, originalRef.current.size);
    onClose();
  }, [onPreview, onClose]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const lowerQuery = query.toLowerCase();
  const filtered = lowerQuery
    ? serverFonts.filter((f) => f.toLowerCase().includes(lowerQuery))
    : serverFonts;

  // Preview the selected font in the terminal
  const previewFont = useCallback((family: string) => {
    onPreview(family, size);
  }, [onPreview, size]);

  // Keyboard navigation
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setSelectedIdx((i) => Math.min(i + 1, filtered.length - 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        setSelectedIdx((i) => Math.max(i - 1, -1));
        break;
      case "Enter":
        e.preventDefault();
        if (selectedIdx >= 0 && selectedIdx < filtered.length) {
          onSelect(filtered[selectedIdx], size);
        } else if (query.trim()) {
          onSelect(query.trim(), size);
        }
        break;
      case "Escape":
        e.preventDefault();
        dismiss();
        break;
    }
  }, [filtered, selectedIdx, query, size, onSelect, dismiss]);

  // Preview on keyboard selection change
  useEffect(() => {
    if (selectedIdx >= 0 && selectedIdx < filtered.length) {
      previewFont(filtered[selectedIdx]);
      const el = listRef.current?.children[selectedIdx] as HTMLElement | undefined;
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIdx, filtered, previewFont]);

  // Reset selection when query changes
  useEffect(() => {
    setSelectedIdx(-1);
  }, [query]);

  const inputStyle = {
    ...styles.exposeSearch,
    backgroundColor: dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.05)",
    color: "inherit",
  };

  const previewFamily = selectedIdx >= 0 && selectedIdx < filtered.length
    ? filtered[selectedIdx]
    : query || currentFamily;

  return (
    <div style={styles.overlay} onClick={dismiss}>
      <section
        style={{
          ...styles.helpBox,
          backgroundColor: dark ? "#1e1e1e" : "#f5f5f5",
          color: dark ? "#e0e0e0" : "#333",
          maxHeight: "80vh",
          display: "flex",
          flexDirection: "column",
          minWidth: 320,
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontWeight: 600, marginBottom: 12, fontSize: 16, flexShrink: 0 }}>Font</h2>
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Search fonts or type a name"
          style={{ ...inputStyle, marginBottom: 8 }}
        />
        {filtered.length > 0 && (
          <ul
            ref={listRef}
            style={{
              margin: 0,
              padding: 0,
              overflow: "auto",
              flex: 1,
              minHeight: 0,
              maxHeight: 200,
              marginBottom: 8,
            }}
          >
            {filtered.map((f, i) => (
              <li
                key={f}
                style={{
                  padding: "4px 8px",
                  cursor: "pointer",
                  borderRadius: 3,
                  backgroundColor: i === selectedIdx
                    ? dark ? "rgba(255,255,255,0.1)" : "rgba(0,0,0,0.08)"
                    : "transparent",
                  listStyle: "none",
                  fontSize: 13,
                }}
                onClick={() => onSelect(f, size)}
                onMouseEnter={() => { setSelectedIdx(i); previewFont(f); }}
              >
                {f}
              </li>
            ))}
          </ul>
        )}
        <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
          <label style={{ fontSize: 13, opacity: 0.7, flexShrink: 0 }}>Size</label>
          <input
            type="range"
            min={8}
            max={32}
            value={size}
            onChange={(e) => setSize(Number(e.target.value))}
            style={{ flex: 1 }}
          />
          <input
            type="number"
            min={6}
            max={72}
            value={size}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (n > 0) setSize(n);
            }}
            style={{ ...inputStyle, width: 52, flex: "none", textAlign: "center" }}
          />
        </div>
        <span style={{ fontSize: size, fontFamily: previewFamily || DEFAULT_FONT, flexShrink: 0, marginTop: 8 }}>
          The quick brown fox
        </span>
      </section>
    </div>
  );
}
