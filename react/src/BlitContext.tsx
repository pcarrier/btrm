import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { BlitTransport, TerminalPalette } from "./types";
import type { TerminalStore } from "./TerminalStore";

export interface BlitContextValue {
  transport?: BlitTransport;
  store?: TerminalStore;
  palette?: TerminalPalette;
  fontFamily?: string;
  fontSize?: number;
}

const BlitContext = createContext<BlitContextValue>({});

export function useBlitContext(): BlitContextValue {
  return useContext(BlitContext);
}

export interface BlitProviderProps extends BlitContextValue {
  children: ReactNode;
}

export function BlitProvider({
  children,
  transport,
  store,
  palette,
  fontFamily,
  fontSize,
}: BlitProviderProps) {
  const value = useMemo(
    () => ({ transport, store, palette, fontFamily, fontSize }),
    [transport, store, palette, fontFamily, fontSize],
  );
  return <BlitContext.Provider value={value}>{children}</BlitContext.Provider>;
}
