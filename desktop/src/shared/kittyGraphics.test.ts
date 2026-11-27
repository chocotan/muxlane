import { describe, expect, it } from "vitest";
import { diacriticIndex, KittyGraphicsScanner } from "./kittyGraphics";

function bytes(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

describe("KittyGraphicsScanner", () => {
  it("strips APC image transmission while preserving surrounding text and chunked m=1 payload", () => {
    const scanner = new KittyGraphicsScanner();
    const b64 = Buffer.from("fake-png-bytes").toString("base64");
    const part1 = b64.slice(0, 8);
    const part2 = b64.slice(8);

    const res1 = scanner.process(bytes(`before\x1b_Ga=t,f=100,i=7,m=1;${part1}\x1b\\mid`));
    expect(new TextDecoder().decode(res1.cleanBytes)).toBe("beforemid");
    expect(res1.updatedImageIds).toEqual([]);

    const res2 = scanner.process(bytes(`\x1b_Gm=0;${part2}\x1b\\after`));
    expect(new TextDecoder().decode(res2.cleanBytes)).toBe("after");
    expect(res2.updatedImageIds).toEqual([7]);
    expect(new TextDecoder().decode(scanner.getImage(7)!.rawBytes)).toBe("fake-png-bytes");
  });

  it("handles tmux-interleaved CSI sequences inside APC without corrupting image payload", () => {
    const scanner = new KittyGraphicsScanner();
    const b64 = Buffer.from("interleaved-ok").toString("base64");
    const seq = `\x1b_Ga=t,f=100,i=9,U=1,c=12,r=6;${b64.slice(0, 6)}\x1b[38;5;9m${b64.slice(6)}\x1b\\`;
    const res = scanner.process(bytes(seq));

    expect(new TextDecoder().decode(res.cleanBytes)).toBe("\x1b[38;5;9m");
    expect(res.updatedImageIds).toEqual([9]);
    expect(scanner.getPlacementSize(9)).toEqual({ cols: 12, rows: 6 });
    expect(new TextDecoder().decode(scanner.getImage(9)!.rawBytes)).toBe("interleaved-ok");
  });

  it("answers Kitty capability queries (a=q) unless suppressed by q=2", () => {
    const scanner = new KittyGraphicsScanner();
    const r1 = scanner.process(bytes("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"));
    expect(r1.replies).toEqual(["\x1b_Gi=31;OK\x1b\\"]);

    const r2 = scanner.process(bytes("\x1b_Gi=32,q=2,a=q;AAAA\x1b\\"));
    expect(r2.replies).toEqual([]);
  });

  it("decodes frozen Kitty row/column diacritic indices", () => {
    expect(diacriticIndex(0x305)).toBe(0);
    expect(diacriticIndex(0x30d)).toBe(1);
    expect(diacriticIndex(0x1d244)).toBe(296);
    expect(diacriticIndex(0x41)).toBeUndefined();
  });
});
