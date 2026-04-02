import { createContext, useContext, type JSX } from "solid-js";
import type { BlitWorkspace, TerminalPalette } from "@blit-sh/core";

export interface BlitContextValue {
  workspace: BlitWorkspace;
  palette?: TerminalPalette;
  fontFamily?: string;
  fontSize?: number;
  advanceRatio?: number;
}

const BlitContext = createContext<BlitContextValue>();

export function useBlitContext(): BlitContextValue {
  const ctx = useContext(BlitContext);
  if (!ctx) {
    throw new Error("Blit components require a BlitWorkspaceProvider ancestor");
  }
  return ctx;
}

export interface BlitProviderProps extends BlitContextValue {
  children: JSX.Element;
}

export function BlitWorkspaceProvider(props: BlitProviderProps) {
  return (
    <BlitContext.Provider
      value={{
        workspace: props.workspace,
        palette: props.palette,
        fontFamily: props.fontFamily,
        fontSize: props.fontSize,
        advanceRatio: props.advanceRatio,
      }}
    >
      {props.children}
    </BlitContext.Provider>
  );
}
