import {
  useState,
  useEffect,
  useRef,
} from "react";
import { DEFAULT_FONT } from "blit-react";
import { styles } from "./styles";

export function FontOverlay({
  current,
  onSelect,
  onClose,
  dark,
}: {
  current: string;
  onSelect: (font: string) => void;
  onClose: () => void;
  dark: boolean;
}) {
  const [value, setValue] = useState(current);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  return (
    <div open style={styles.overlay} onClick={onClose}>
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
            onSelect(value);
          }}
          style={{ display: "flex", flexDirection: "column", gap: 10 }}
        >
          <input
            ref={inputRef}
            type="text"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder="Font family (CSS value)"
            style={{
              ...styles.exposeSearch,
              backgroundColor: dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.05)",
              color: "inherit",
            }}
          />
          <span style={{ fontSize: 13, opacity: 0.6 }}>
            Preview: <span style={{ fontFamily: value || DEFAULT_FONT }}>The quick brown fox jumps over the lazy dog</span>
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
