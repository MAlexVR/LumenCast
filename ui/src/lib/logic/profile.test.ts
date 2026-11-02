import { describe, expect, it } from "vitest";
import { profileFileName } from "./profile";

describe("profileFileName", () => {
  it("is dated, zero padded and ends in .json", () => {
    expect(profileFileName(new Date(2026, 10, 2))).toBe("lumencast-profile-2026-11-02.json");
    expect(profileFileName(new Date(2027, 0, 9))).toBe("lumencast-profile-2027-01-09.json");
  });
});
