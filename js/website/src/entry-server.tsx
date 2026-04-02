import { renderToString } from "react-dom/server";
import { Landing } from "./Landing";

export function render(): string {
  return renderToString(<Landing theme="dark" onToggleTheme={() => {}} />);
}
