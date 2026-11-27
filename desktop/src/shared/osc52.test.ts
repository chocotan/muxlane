import { describe, expect, it } from "vitest";
import { parseOsc52Payload } from "./osc52";

function b64(text: string): string {
  return Buffer.from(text, "utf8").toString("base64");
}

describe("parseOsc52Payload", () => {
  it("decodes standard ASCII clipboard write from tmux", () => {
    const parsed = parseOsc52Payload(`c;${b64("copied-from-remote-tmux")}`);
    expect(parsed).toEqual({
      selection: "c",
      text: "copied-from-remote-tmux",
      alsoSelection: true,
    });
  });

  it("decodes multibyte UTF-8 (Chinese and emoji) without mojibake", () => {
    const sample = "远程机器复制测试 🚀\n第二行";
    const parsed = parseOsc52Payload(`c;${b64(sample)}`);
    expect(parsed?.text).toBe(sample);
  });

  it("handles empty selection, primary selection, unpadded base64, and wrapped base64", () => {
    const rawB64 = b64("hello world!");
    expect(parseOsc52Payload(`;${rawB64}`)?.text).toBe("hello world!");
    expect(parseOsc52Payload(`p;${rawB64}`)?.selection).toBe("p");
    expect(parseOsc52Payload(`c;${rawB64.replace(/=+$/, "")}`)?.text).toBe("hello world!");
    expect(parseOsc52Payload(`c;${rawB64.slice(0, 4)}\r\n${rawB64.slice(4)}`)?.text).toBe(
      "hello world!",
    );
  });

  it("ignores clipboard read queries, empty clears, and invalid inputs", () => {
    expect(parseOsc52Payload("c;?")).toBeNull();
    expect(parseOsc52Payload(";?")).toBeNull();
    expect(parseOsc52Payload("c;")).toBeNull();
    expect(parseOsc52Payload("no-semicolon")).toBeNull();
    expect(parseOsc52Payload("c;!!!not-base64!!!")).toBeNull();
  });

  it("rejects oversized payloads beyond limit", () => {
    const big = "a".repeat(129);
    expect(parseOsc52Payload(`c;${b64(big)}`, 128)).toBeNull();
    expect(parseOsc52Payload(`c;${b64("a".repeat(128))}`, 128)?.text.length).toBe(128);
  });
});
