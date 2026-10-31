import { describe, expect, it } from "vitest";
import {
  calendarDate,
  dayNumber,
  eventOfDay,
  holidayOf,
  solarTermOf,
  upcomingSolarTerm,
} from "./calendar";

const on = (year: number, month: number, day: number) =>
  new Date(year, month - 1, day);

describe("日本の祝日", () => {
  it("2026年の固定祝日を判定する", () => {
    expect(holidayOf(on(2026, 1, 1))?.name).toBe("元日");
    expect(holidayOf(on(2026, 1, 12))?.name).toBe("成人の日");
    expect(holidayOf(on(2026, 2, 11))?.name).toBe("建国記念の日");
    expect(holidayOf(on(2026, 2, 23))?.name).toBe("天皇誕生日");
    expect(holidayOf(on(2026, 4, 29))?.name).toBe("昭和の日");
    expect(holidayOf(on(2026, 5, 3))?.name).toBe("憲法記念日");
    expect(holidayOf(on(2026, 5, 4))?.name).toBe("みどりの日");
    expect(holidayOf(on(2026, 5, 5))?.name).toBe("こどもの日");
    expect(holidayOf(on(2026, 8, 11))?.name).toBe("山の日");
    expect(holidayOf(on(2026, 9, 21))?.name).toBe("敬老の日");
    expect(holidayOf(on(2026, 10, 12))?.name).toBe("スポーツの日");
    expect(holidayOf(on(2026, 11, 3))?.name).toBe("文化の日");
    expect(holidayOf(on(2026, 11, 23))?.name).toBe("勤労感謝の日");
  });

  it("ハッピーマンデーとなる祝日を第 n 月曜日で求める", () => {
    // 2026年9月1日は火曜日なので、敬老の日は第3月曜日の21日。
    expect(holidayOf(on(2026, 9, 21))?.kind).toBe("holiday");
    expect(holidayOf(on(2026, 9, 14))).toBeNull();
    expect(holidayOf(on(2026, 10, 12))?.kind).toBe("holiday");
  });

  it("日曜の祝日は直後の非祝日へ振替する", () => {
    // 2026年5月3日は日曜。5月4日と5日も祝日なので振替は5月6日。
    expect(holidayOf(on(2026, 5, 6))?.name).toBe("振替休日");
    expect(holidayOf(on(2026, 5, 6))?.kind).toBe("substitute");
    expect(holidayOf(on(2026, 5, 7))).toBeNull();
  });

  it("祝日に挟まれた非日曜・非月曜の日を国民の休日とする", () => {
    // 2026年9月21日が敬老の日、9月23日が秋分の日で、9月22日を挟む。
    expect(holidayOf(on(2026, 9, 22))?.name).toBe("国民の休日");
    expect(holidayOf(on(2026, 9, 22))?.kind).toBe("citizen");
  });

  it("祝日でない日は null を返す", () => {
    expect(holidayOf(on(2026, 6, 10))).toBeNull();
    expect(holidayOf(on(2026, 2, 10))).toBeNull();
  });

  it("春分の日と秋分の日を近似式で求める", () => {
    expect(holidayOf(on(2026, 3, 20))?.name).toBe("春分の日");
    expect(holidayOf(on(2026, 9, 23))?.name).toBe("秋分の日");
  });
});

describe("二十四節気", () => {
  // 国立天文台「暦要項 令和8年(2026)」の中央標準時表記と一致することを確認した値。
  it("2026年の二十四節気すべてが官報値と一致する", () => {
    const expected: readonly (readonly [number, number, string])[] = [
      [1, 5, "小寒"],
      [1, 20, "大寒"],
      [2, 4, "立春"],
      [2, 19, "雨水"],
      [3, 5, "啓蟄"],
      [3, 20, "春分"],
      [4, 5, "清明"],
      [4, 20, "穀雨"],
      [5, 5, "立夏"],
      [5, 21, "小満"],
      [6, 6, "芒種"],
      [6, 21, "夏至"],
      [7, 7, "小暑"],
      [7, 23, "大暑"],
      [8, 7, "立秋"],
      [8, 23, "処暑"],
      [9, 7, "白露"],
      [9, 23, "秋分"],
      [10, 8, "寒露"],
      [10, 23, "霜降"],
      [11, 7, "立冬"],
      [11, 22, "小雪"],
      [12, 7, "大雪"],
      [12, 22, "冬至"],
    ];
    for (const [month, day, name] of expected)
      expect(solarTermOf(on(2026, month, day))?.name, `${month}/${day}`).toBe(
        name,
      );
  });

  it("夜半を跨ぐ黄経の到達は当日と判定する", () => {
    // 2026年の春分は3月20日23時46分。日正午で比較すると21日になるため、
    // 0 時単位の区間判定になっていることを確認する。
    expect(solarTermOf(on(2026, 3, 19))).toBeNull();
    expect(solarTermOf(on(2026, 3, 21))).toBeNull();
    expect(solarTermOf(on(2026, 3, 20))?.name).toBe("春分");
  });

  it("次の節気を日数つきで返す", () => {
    expect(upcomingSolarTerm(on(2026, 3, 21))).toEqual({
      name: "清明",
      inDays: 15,
    });
    expect(upcomingSolarTerm(on(2026, 10, 1))).toEqual({
      name: "寒露",
      inDays: 7,
    });
  });
});

describe("季節の行事", () => {
  it("固定日の行事を返す", () => {
    expect(eventOfDay(on(2026, 2, 14))?.name).toBe("バレンタインデー");
    expect(eventOfDay(on(2026, 12, 25))?.name).toBe("クリスマス");
    expect(eventOfDay(on(2026, 12, 31))?.name).toBe("大晦日");
    expect(eventOfDay(on(2026, 7, 7))?.name).toBe("七夕");
  });

  it("祝日と重なる日は行事を返さない", () => {
    expect(eventOfDay(on(2026, 5, 5))).toBeNull();
    expect(eventOfDay(on(2026, 1, 1))).toBeNull();
  });

  it("行事がない日は null を返す", () => {
    expect(eventOfDay(on(2026, 6, 10))).toBeNull();
  });
});

describe("日付の補助", () => {
  it("ローカル日付を日番号へ変換する", () => {
    expect(dayNumber(1970, 1, 1)).toBe(0);
    expect(dayNumber(2026, 1, 1) - dayNumber(2025, 12, 31)).toBe(1);
  });

  it("Date からローカル年月日を取り出す", () => {
    expect(calendarDate(on(2026, 2, 4))).toEqual({
      year: 2026,
      month: 2,
      day: 4,
    });
  });
});
