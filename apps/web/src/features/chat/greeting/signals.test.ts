import { describe, expect, it } from "vitest";
import {
  activityOf,
  buildSignals,
  dayKind,
  dayOfYear,
  gapOf,
  partOfDay,
  seasonOf,
  workloadOf,
} from "./signals";

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const base = new Date(2026, 5, 10, 12, 0);

describe("時間帯の判定", () => {
  it("8 区分の境界を正しく扱う", () => {
    expect(partOfDay(new Date(2026, 0, 1, 3))).toBe("lateNight");
    expect(partOfDay(new Date(2026, 0, 1, 5))).toBe("earlyMorning");
    expect(partOfDay(new Date(2026, 0, 1, 7))).toBe("morning");
    expect(partOfDay(new Date(2026, 0, 1, 9))).toBe("lateMorning");
    expect(partOfDay(new Date(2026, 0, 1, 12))).toBe("midday");
    expect(partOfDay(new Date(2026, 0, 1, 14))).toBe("afternoon");
    expect(partOfDay(new Date(2026, 0, 1, 17))).toBe("dusk");
    expect(partOfDay(new Date(2026, 0, 1, 19))).toBe("evening");
    expect(partOfDay(new Date(2026, 0, 1, 22))).toBe("lateNight");
    expect(partOfDay(new Date(2026, 0, 1, 23))).toBe("lateNight");
  });
});

describe("曜日と季節", () => {
  it("曜日を 5 種類に分ける", () => {
    expect(dayKind(new Date(2026, 5, 7))).toBe("sunday");
    expect(dayKind(new Date(2026, 5, 6))).toBe("weekend");
    expect(dayKind(new Date(2026, 5, 8))).toBe("monday");
    expect(dayKind(new Date(2026, 5, 12))).toBe("friday");
    expect(dayKind(new Date(2026, 5, 10))).toBe("weekday");
  });

  it("季節と季内の位置を返す", () => {
    // 3月-5月は春、6月-8月は夏、9月-11月は秋、12月-2月は冬。
    expect(seasonOf(new Date(2026, 2, 15))).toEqual({
      season: "spring",
      seasonPhase: "early",
    });
    expect(seasonOf(new Date(2026, 4, 15))).toEqual({
      season: "spring",
      seasonPhase: "late",
    });
    expect(seasonOf(new Date(2026, 6, 15))).toEqual({
      season: "summer",
      seasonPhase: "mid",
    });
    expect(seasonOf(new Date(2026, 7, 15))).toEqual({
      season: "summer",
      seasonPhase: "late",
    });
    expect(seasonOf(new Date(2026, 8, 15))).toEqual({
      season: "autumn",
      seasonPhase: "early",
    });
    expect(seasonOf(new Date(2026, 9, 15))).toEqual({
      season: "autumn",
      seasonPhase: "mid",
    });
    expect(seasonOf(new Date(2026, 0, 15))).toEqual({
      season: "winter",
      seasonPhase: "mid",
    });
    expect(seasonOf(new Date(2026, 11, 15))).toEqual({
      season: "winter",
      seasonPhase: "early",
    });
  });

  it("年内の日数を返す", () => {
    expect(dayOfYear(new Date(2026, 0, 1))).toBe(1);
    expect(dayOfYear(new Date(2026, 11, 31))).toBe(365);
    expect(dayOfYear(new Date(2028, 11, 31))).toBe(366);
  });
});

describe("ワークスペースの規模", () => {
  it("会話数から 4 段階に分ける", () => {
    expect(workloadOf(0)).toBe("empty");
    expect(workloadOf(4)).toBe("light");
    expect(workloadOf(19)).toBe("steady");
    expect(workloadOf(20)).toBe("heavy");
  });
});

describe("最終活動からの経過", () => {
  it("会話が無い場合は none", () => {
    expect(activityOf(null, base)).toBe("none");
  });

  it("経過時間から 6 段階に分ける", () => {
    expect(activityOf(base.getTime() - 10 * 60_000, base)).toBe("justNow");
    expect(activityOf(base.getTime() - 5 * HOUR, base)).toBe("today");
    expect(activityOf(base.getTime() - 26 * HOUR, base)).toBe("yesterday");
    expect(activityOf(base.getTime() - 3 * DAY, base)).toBe("thisWeek");
    expect(activityOf(base.getTime() - 30 * DAY, base)).toBe("earlier");
  });
});

describe("訪問間隔", () => {
  it("履歴が無い場合は初回", () => {
    expect(gapOf(base, 0)).toBe("first");
  });

  it("経過時間から 7 段階に分ける", () => {
    expect(gapOf(base, base.getTime() - 60_000)).toBe("reload");
    expect(gapOf(base, base.getTime() - 6 * HOUR)).toBe("sameDay");
    expect(gapOf(base, base.getTime() - 26 * HOUR)).toBe("overnight");
    expect(gapOf(base, base.getTime() - 3 * DAY)).toBe("fewDays");
    expect(gapOf(base, base.getTime() - 14 * DAY)).toBe("longAway");
    expect(gapOf(base, base.getTime() - 90 * DAY)).toBe("farAway");
  });
});

describe("シグナルの組み立て", () => {
  it("日時と暦から状態を組み立てる", () => {
    // 2026年5月5日はこどもの日で、午後。
    const signals = buildSignals({
      now: new Date(2026, 4, 5, 15, 0),
      conversationCount: 30,
      pinnedCount: 3,
      projectCount: 2,
      agentCount: 1,
      lastMessageAt: new Date(2026, 4, 5, 14, 0).getTime(),
      lastVisitAt: 0,
      firstSeenTodayAt: 0,
      visits: 0,
    });
    expect(signals.partOfDay).toBe("afternoon");
    expect(signals.dayKind).toBe("weekday");
    expect(signals.season).toBe("spring");
    expect(signals.holiday).toBe("こどもの日");
    expect(signals.event).toBeNull();
    expect(signals.activity).toBe("today");
    expect(signals.gap).toBe("first");
    expect(signals.conversationCount).toBe(30);
    expect(signals.firstToday).toBe(true);
  });

  it("年の出だしと終わりの時期を区別する", () => {
    const at = (now: Date) =>
      buildSignals({
        now,
        conversationCount: 0,
        pinnedCount: 0,
        projectCount: 0,
        agentCount: 0,
        lastMessageAt: null,
        lastVisitAt: 0,
        firstSeenTodayAt: 0,
        visits: 0,
      }).yearProgress;
    expect(at(new Date(2026, 0, 3))).toBe("start");
    expect(at(new Date(2026, 5, 10))).toBeNull();
    expect(at(new Date(2026, 11, 28))).toBe("end");
  });

  it("その日の最初の訪問だけを firstToday とする", () => {
    const now = new Date(2026, 5, 10, 12, 0);
    const midnight = new Date(2026, 5, 10).getTime();
    const build = (firstSeenTodayAt: number) =>
      buildSignals({
        now,
        conversationCount: 0,
        pinnedCount: 0,
        projectCount: 0,
        agentCount: 0,
        lastMessageAt: null,
        lastVisitAt: midnight + HOUR,
        firstSeenTodayAt,
        visits: 1,
      }).firstToday;
    expect(build(0)).toBe(true);
    expect(build(midnight + HOUR)).toBe(false);
    expect(build(midnight - DAY)).toBe(true);
  });
});
