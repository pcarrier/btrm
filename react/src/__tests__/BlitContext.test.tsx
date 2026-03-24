import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { BlitProvider, useBlitContext } from "../BlitContext";
import type { BlitContextValue } from "../BlitContext";
import { MockTransport } from "./mock-transport";

function wrapper(value: BlitContextValue) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <BlitProvider {...value}>{children}</BlitProvider>;
  };
}

describe("BlitContext", () => {
  it("returns empty object without provider", () => {
    const { result } = renderHook(() => useBlitContext());
    expect(result.current).toEqual({});
  });

  it("provides transport", () => {
    const transport = new MockTransport();
    const { result } = renderHook(() => useBlitContext(), {
      wrapper: wrapper({ transport }),
    });
    expect(result.current.transport).toBe(transport);
  });

  it("provides palette", () => {
    const palette = {
      id: "test",
      name: "Test",
      dark: true,
      fg: [255, 255, 255] as [number, number, number],
      bg: [0, 0, 0] as [number, number, number],
      ansi: Array.from({ length: 16 }, () => [0, 0, 0] as [number, number, number]),
    };
    const { result } = renderHook(() => useBlitContext(), {
      wrapper: wrapper({ palette }),
    });
    expect(result.current.palette).toBe(palette);
  });

  it("provides fontFamily and fontSize", () => {
    const { result } = renderHook(() => useBlitContext(), {
      wrapper: wrapper({ fontFamily: "monospace", fontSize: 14 }),
    });
    expect(result.current.fontFamily).toBe("monospace");
    expect(result.current.fontSize).toBe(14);
  });

  it("provides undefined for omitted values", () => {
    const { result } = renderHook(() => useBlitContext(), {
      wrapper: wrapper({}),
    });
    expect(result.current.transport).toBeUndefined();
    expect(result.current.store).toBeUndefined();
    expect(result.current.palette).toBeUndefined();
    expect(result.current.fontFamily).toBeUndefined();
    expect(result.current.fontSize).toBeUndefined();
  });
});
