import { onMount, onCleanup, createEffect } from "solid-js";
import type { JSX } from "solid-js";
import { TerminalView, DEFAULT_FONT, DEFAULT_FONT_SIZE } from "@blit-sh/core";
import type { SessionId, TerminalPalette } from "@blit-sh/core";
import { useBlitContext } from "./BlitContext";

export interface BlitTerminalProps {
  sessionId: SessionId | null;
  fontFamily?: string;
  fontSize?: number;
  class?: string;
  style?: JSX.CSSProperties | string;
  palette?: TerminalPalette;
  readOnly?: boolean;
  showCursor?: boolean;
  onRender?: (renderMs: number) => void;
  scrollbarColor?: string;
  scrollbarWidth?: number;
  advanceRatio?: number;
  ref?: (view: TerminalView) => void;
}

export function BlitTerminal(props: BlitTerminalProps) {
  const ctx = useBlitContext();
  let containerEl!: HTMLDivElement;
  let view: TerminalView | undefined;

  onMount(() => {
    view = new TerminalView({
      container: containerEl,
      workspace: ctx.workspace,
      sessionId: props.sessionId,
      fontFamily: props.fontFamily ?? ctx.fontFamily ?? DEFAULT_FONT,
      fontSize: props.fontSize ?? ctx.fontSize ?? DEFAULT_FONT_SIZE,
      palette: props.palette ?? ctx.palette,
      readOnly: props.readOnly,
      showCursor: props.showCursor,
      scrollbarColor: props.scrollbarColor,
      scrollbarWidth: props.scrollbarWidth,
      advanceRatio: props.advanceRatio ?? ctx.advanceRatio,
      onRender: props.onRender,
    });
    props.ref?.(view);
  });

  onCleanup(() => view?.dispose());

  createEffect(() => { view && (view.sessionId = props.sessionId); });
  createEffect(() => { view && (view.palette = props.palette ?? ctx.palette); });
  createEffect(() => { if (view && props.showCursor !== undefined) view.showCursor = props.showCursor; });
  createEffect(() => { if (view && props.readOnly !== undefined) view.readOnly = props.readOnly; });
  createEffect(() => { view && (view.fontFamily = props.fontFamily ?? ctx.fontFamily ?? DEFAULT_FONT); });
  createEffect(() => { view && (view.fontSize = props.fontSize ?? ctx.fontSize ?? DEFAULT_FONT_SIZE); });
  createEffect(() => { if (view && props.advanceRatio !== undefined) view.advanceRatio = props.advanceRatio; });
  createEffect(() => { if (view && props.scrollbarColor) view.scrollbarColor = props.scrollbarColor; });
  createEffect(() => { if (view && props.scrollbarWidth !== undefined) view.scrollbarWidth = props.scrollbarWidth; });
  createEffect(() => { if (view) view.onRender = props.onRender; });

  return <div ref={containerEl} class={props.class} style={props.style} />;
}
