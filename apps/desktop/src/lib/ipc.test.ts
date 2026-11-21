import { describe, expect, it } from "vitest";
import { errorText } from "./ipc";

describe("errorText", () => {
  it("formats structured core errors", () => {
    expect(errorText({ code: "ROUTE_STALE", message: "route gone" })).toBe("ROUTE_STALE: route gone");
  });
  it("falls back to string conversion", () => {
    expect(errorText("boom")).toBe("boom");
  });
});
