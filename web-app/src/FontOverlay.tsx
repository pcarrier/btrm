import {
  useState,
  useEffect,
  useRef,
} from "react";
import { DEFAULT_FONT } from "blit-react";
import { styles } from "./styles";

export function FontOverlay({
  currentFamily,
  currentSize,
  onSelect,
  onClose,
  dark,
}: {
  currentFamily: string;
  currentSize: number;
  onSelect: (font: string, size: number) => void;
  onClose: () => void;
  dark: boolean;
}) {
  const [family, setFamily] = useState(currentFamily);
  const [size, setSize] = useState(currentSize);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const inputStyle = {
    ...styles.exposeSearch,
    backgroundColor: dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.05)",
    color: "inherit",
  };

  return (
    <div style={styles.overlay} onClick={onClose}>
      <section
        style={{
          ...styles.helpBox,
          backgroundColor: dark ? "#1e1e1e" : "#f5f5f5",
          color: dark ? "#e0e0e0" : "#333",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontWeight: 600, marginBottom: 12, fontSize: 16 }}>Font</h2>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            onSelect(family, size);
          }}
          style={{ display: "flex", flexDirection: "column", gap: 10 }}
        >
          <input
            ref={inputRef}
            type="text"
            value={family}
            onChange={(e) => setFamily(e.target.value)}
            placeholder="Font family (CSS value)"
            style={inputStyle}
          />
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
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
          <span style={{ fontSize: size, fontFamily: family || DEFAULT_FONT, opacity: 0.6 }}>
            The quick brown fox
          </span>
          <button
            type="submit"
            style={{
              ...styles.statusBtn,
              alignSelf: "flex-end",
              padding: "4px 12px",
              border: "1px solid rgba(128,128,128,0.3)",
            }}
          >
            Apply
          </button>
        </form>
      </section>
    </div>
  );
}
