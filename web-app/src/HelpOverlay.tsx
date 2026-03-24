import { styles } from "./styles";

export function HelpOverlay({
  onClose,
  dark,
}: {
  onClose: () => void;
  dark: boolean;
}) {
  const mod_ = /Mac|iPhone|iPad/.test(navigator.platform) ? "Cmd" : "Ctrl";
  const shortcuts = [
    [`${mod_}+K`, "Expose (switch PTYs)"],
    [`${mod_}+Shift+Enter`, "New PTY in cwd"],
    [`${mod_}+Shift+W`, "Close PTY"],
    [`${mod_}+Shift+{ / }`, "Prev / Next PTY"],
    ["Shift+PageUp/Down", "Scroll"],
    [`${mod_}+Shift+P`, "Palette picker"],
    [`${mod_}+Shift+F`, "Font picker"],
    ["Ctrl+?", "Help"],
  ];
  return (
    <div
      open
      style={styles.overlay}
      onClick={onClose}
    >
      <article
        style={{
          ...styles.helpBox,
          backgroundColor: dark ? "#1e1e1e" : "#f5f5f5",
          color: dark ? "#e0e0e0" : "#333",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontWeight: 600, marginBottom: 12, fontSize: 16 }}>
          Keyboard shortcuts
        </h2>
        <table style={{ borderSpacing: "12px 6px" }}>
          <tbody>
            {shortcuts.map(([key, desc]) => (
              <tr key={key}>
                <td>
                  <kbd style={styles.kbd}>{key}</kbd>
                </td>
                <td style={{ fontSize: 13 }}>{desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </article>
    </div>
  );
}
