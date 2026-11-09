/**
 * **ずらしの帯の文**（`src/lib/portShift.ts` / 作業単位 F-11c / ADR-0057 §決定4）。
 *
 * 帯と `App.svelte` の配線は `tests/port-shift-notice.test.ts` が見る。ここは文の枝の全部。
 */

import { describe, expect, it } from "vitest";

import type { ServerView } from "../src/lib/api";
import { portShiftNotice } from "../src/lib/portShift";

/** ずらさずに保存した番号で起きた形（**番号はどれも違える**——取り違えが観測できるように）。 */
function viewOf(overrides: Partial<ServerView> = {}): ServerView {
  return {
    listening_port: 9102,
    listening_port_source: "saved",
    listening_port_requested: 9100,
    listening_port_shifted: true,
    listening_port_skipped: [9101],
    saved_port: 9100,
    saved_port_unreadable: false,
    next_port: 9100,
    next_port_source: "saved",
    editable: true,
    ...overrides,
  };
}

/** 帯に出る文（リンクの文言は「設定」）。 */
function sentence(view: ServerView): string | null {
  const notice = portShiftNotice(view);
  return notice === null ? null : `${notice.lead}設定${notice.tail}`;
}

describe("ずらしの帯の文", () => {
  /** **「使っていた」と言い切らない**（`PermissionDenied` も含む / ADR-0057 §決定4 の追記）。 */
  it("開けなかった元の番号は「使っているなどの理由で使えなかった」と言い、直すきっかけと「設定」を言う", () => {
    expect(sentence(viewOf())).toBe(
      "ポート 9100 は他のプログラムが使っているなどの理由で使えなかったので、9102 で起動しました。" +
        "ブックマークや外部のクライアントに 9100 を書いているなら、9102 に直してください。" +
        "番号は「設定」の画面で変えられます（次の起動から効きます）。",
    );
  });

  it("面の番号として飛ばした元の番号には「他のプログラム」「使えなかった」と言わない", () => {
    const text = sentence(viewOf({ listening_port: 9101, listening_port_skipped: [9100] }));
    expect(text).toBe(
      "ポート 9100 は上流プロファイルの llama の面が使う番号なので、9101 で起動しました。" +
        "ブックマークや外部のクライアントに 9100 を書いているなら、9101 に直してください。" +
        "番号は「設定」の画面で変えられます（次の起動から効きます）。",
    );
    expect(text).not.toContain("他のプログラム");
    expect(text).not.toContain("使えなかった");
  });

  it("元の番号の後で面の番号を飛ばした形は、開けなかった方の文", () => {
    expect(sentence(viewOf({ listening_port_skipped: [9101] }))).toContain(
      "ポート 9100 は他のプログラムが使っているなどの理由で使えなかったので、",
    );
  });

  it("ずらしていなければ出さない（OS に選ばせた 0 の起動も）", () => {
    expect(
      portShiftNotice(
        viewOf({
          listening_port: 9100,
          listening_port_shifted: false,
          listening_port_skipped: [],
        }),
      ),
    ).toBeNull();
    expect(
      portShiftNotice(
        viewOf({
          listening_port: 51234,
          listening_port_source: "environment",
          listening_port_requested: 0,
          listening_port_shifted: false,
          listening_port_skipped: [],
        }),
      ),
    ).toBeNull();
  });
});
