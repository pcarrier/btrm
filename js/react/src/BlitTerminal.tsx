import { useEffect, useImperativeHandle, useRef } from "react";
import type { Ref } from "react";
import type { Terminal } from "@blit-sh/browser";
import type { ConnectionStatus } from "@blit-sh/core";
import { TerminalView, DEFAULT_FONT, DEFAULT_FONT_SIZE } from "@blit-sh/core";
import type { BlitTerminalProps } from "./types";
import { useBlitContext, useRequiredBlitWorkspace } from "./BlitContext";

export interface BlitTerminalHandle {
  terminal: Terminal | null;
  rows: number;
  cols: number;
  status: ConnectionStatus;
  focus(): void;
}

export function BlitTerminal({
  ref,
  ...props
}: BlitTerminalProps & { ref?: Ref<BlitTerminalHandle> }) {
  const ctx = useBlitContext();
  const workspace = useRequiredBlitWorkspace();
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<TerminalView | null>(null);

  const {
    sessionId,
    fontFamily = ctx.fontFamily ?? DEFAULT_FONT,
    fontSize = ctx.fontSize ?? DEFAULT_FONT_SIZE,
    className,
    style,
    palette = ctx.palette,
    readOnly = false,
    showCursor = true,
    onRender,
    scrollbarColor = "rgba(255,255,255,0.3)",
    scrollbarWidth = 4,
    advanceRatio = ctx.advanceRatio,
  } = props;

  useEffect(() => {
    const view = new TerminalView({
      container: containerRef.current!,
      workspace,
      sessionId,
      fontFamily,
      fontSize,
      palette,
      readOnly,
      showCursor,
      scrollbarColor,
      scrollbarWidth,
      advanceRatio,
      onRender,
    });
    viewRef.current = view;
    return () => {
      view.dispose();
      viewRef.current = null;
    };
  }, [workspace]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.sessionId = sessionId;
  }, [sessionId]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.palette = palette;
  }, [palette]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.showCursor = showCursor;
  }, [showCursor]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.readOnly = readOnly;
  }, [readOnly]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.fontFamily = fontFamily;
  }, [fontFamily]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.fontSize = fontSize;
  }, [fontSize]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.advanceRatio = advanceRatio;
  }, [advanceRatio]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.scrollbarColor = scrollbarColor;
  }, [scrollbarColor]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.scrollbarWidth = scrollbarWidth;
  }, [scrollbarWidth]);

  useEffect(() => {
    if (viewRef.current) viewRef.current.onRender = onRender;
  }, [onRender]);

  useImperativeHandle(
    ref,
    () => ({
      get terminal() {
        return viewRef.current?.currentTerminal ?? null;
      },
      get rows() {
        return viewRef.current?.rows ?? 24;
      },
      get cols() {
        return viewRef.current?.cols ?? 80;
      },
      get status() {
        return viewRef.current?.status ?? "disconnected";
      },
      focus() {
        viewRef.current?.focus();
      },
    }),
    [],
  );

  return <div ref={containerRef} className={className} style={style} />;
}
