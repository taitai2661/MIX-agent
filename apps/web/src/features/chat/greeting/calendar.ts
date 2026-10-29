const MS_DAY = 86_400_000;
const RAD = Math.PI / 180;
const J1970 = 2_440_587.5;

export type EntryKind = "holiday" | "substitute" | "citizen" | "event" | "term";

export interface Entry {
  name: string;
  kind: EntryKind;
}

function mod360(value: number): number {
  return ((value % 360) + 360) % 360;
}

/** Local calendar date as a day index, independent of any DST rules. */
export function dayNumber(year: number, month: number, day: number): number {
  return Math.round(Date.UTC(year, month - 1, day) / MS_DAY);
}

export function calendarDate(date: Date): {
  year: number;
  month: number;
  day: number;
} {
  return {
    year: date.getFullYear(),
    month: date.getMonth() + 1,
    day: date.getDate(),
  };
}

/** 日番号の日の日本標準時 0:00 (= 前日 15:00 UTC) をユリウス日へ。 */
function julianDayAtJstMidnight(number: number): number {
  return J1970 + number - 0.375;
}

function solarLongitude(jd: number): number {
  const t = (jd - 2_451_545.0) / 36_525;
  const meanLongitude = 280.46646 + 36_000.76983 * t + 0.0003032 * t * t;
  const anomaly = 357.52911 + 35_999.05029 * t - 0.0001537 * t * t;
  const center =
    (1.914602 - 0.004817 * t - 0.000014 * t * t) * Math.sin(anomaly * RAD) +
    (0.019993 - 0.000101 * t) * Math.sin(2 * anomaly * RAD) +
    0.000289 * Math.sin(3 * anomaly * RAD);
  const omega = 125.04 - 1934.136 * t;
  return mod360(
    meanLongitude + center - 0.00569 - 0.00478 * Math.sin(omega * RAD),
  );
}

const SOLAR_TERMS: readonly (readonly [number, string])[] = [
  [270, "冬至"],
  [285, "小寒"],
  [300, "大寒"],
  [315, "立春"],
  [330, "雨水"],
  [345, "啓蟄"],
  [0, "春分"],
  [15, "清明"],
  [30, "穀雨"],
  [45, "立夏"],
  [60, "小満"],
  [75, "芒種"],
  [90, "夏至"],
  [105, "小暑"],
  [120, "大暑"],
  [135, "立秋"],
  [150, "処暑"],
  [165, "白露"],
  [180, "秋分"],
  [195, "寒露"],
  [210, "霜降"],
  [225, "立冬"],
  [240, "小雪"],
  [255, "大雪"],
];

const termCache = new Map<number, Map<string, number>>();

/**
 * 各二十四節気（平太陽視黄経 15° 毎）の日付を、その年に属する日番号で返します。
 * 黄経は推定式から日本標準時の 0 時ごとに計算します。節気となる日は
 * 「その日の 0 時より前、次の日の 0 時以降」に黄経が目標へ届く日なので、
 * 日正午で比較すると夜半を跨ぐ節気が翌日にずれます。
 */
function termsOfYear(year: number): Map<string, number> {
  const cached = termCache.get(year);
  if (cached) return cached;
  const start = dayNumber(year, 1, 1);
  const end = dayNumber(year, 12, 31);
  const chain: number[] = [];
  let previous = solarLongitude(julianDayAtJstMidnight(start));
  chain.push(previous);
  for (let number = start + 1; number <= end + 1; number++) {
    const raw = solarLongitude(julianDayAtJstMidnight(number));
    const value = raw + 360 * Math.round((previous - raw) / 360);
    chain.push(value);
    previous = value;
  }
  const result = new Map<string, number>();
  for (const [target, name] of SOLAR_TERMS) {
    let goal = target;
    while (goal < chain[0]) goal += 360;
    let found = end;
    for (let index = 0; index < chain.length - 1; index++) {
      if (chain[index + 1] < goal) continue;
      found = start + index;
      break;
    }
    result.set(name, found);
  }
  termCache.set(year, result);
  return result;
}

function nthWeekday(
  year: number,
  month: number,
  weekday: number,
  nth: number,
): number {
  const first = new Date(Date.UTC(year, month - 1, 1)).getUTCDay();
  return 1 + ((weekday - first + 7) % 7) + (nth - 1) * 7;
}

/** 春分の日 / 秋分の日の近似式（1888年〜2099年）。 */
function equinoxDay(year: number, autumn: boolean): number {
  const offset = year - 1980;
  if (offset < 0 || offset > 119) return autumn ? 23 : 20;
  const base = autumn ? 23.2488 : 20.8431;
  return Math.floor(base + 0.242194 * offset) - Math.floor(offset / 4);
}

const holidayCache = new Map<number, Map<number, Entry>>();

function holidaysOfYear(year: number): Map<number, Entry> {
  const cached = holidayCache.get(year);
  if (cached) return cached;
  const fixed = new Map<number, Entry>();
  const add = (month: number, day: number, name: string) =>
    fixed.set(dayNumber(year, month, day), { name, kind: "holiday" });
  add(1, 1, "元日");
  add(1, nthWeekday(year, 1, 1, 2), "成人の日");
  add(2, 11, "建国記念の日");
  add(2, 23, "天皇誕生日");
  add(3, equinoxDay(year, false), "春分の日");
  add(4, 29, "昭和の日");
  add(5, 3, "憲法記念日");
  add(5, 4, "みどりの日");
  add(5, 5, "こどもの日");
  add(8, 11, "山の日");
  add(9, nthWeekday(year, 9, 1, 3), "敬老の日");
  add(9, equinoxDay(year, true), "秋分の日");
  add(10, nthWeekday(year, 10, 1, 2), "スポーツの日");
  add(11, 3, "文化の日");
  add(11, 23, "勤労感謝の日");

  const isSunday = (number: number) =>
    new Date(number * MS_DAY).getUTCDay() === 0;
  const result = new Map(fixed);
  for (const [number] of fixed) {
    if (!isSunday(number)) continue;
    let target = number + 1;
    while (result.has(target) || isSunday(target)) target++;
    result.set(target, { name: "振替休日", kind: "substitute" });
  }
  for (
    let number = dayNumber(year, 1, 3);
    number <= dayNumber(year, 12, 30);
    number++
  ) {
    if (result.has(number)) continue;
    if (new Date(number * MS_DAY).getUTCDay() === 1) continue;
    const before = result.get(number - 1);
    const after = result.get(number + 1);
    if (!before || !after) continue;
    if (before.kind === "substitute" || after.kind === "substitute") continue;
    result.set(number, { name: "国民の休日", kind: "citizen" });
  }
  holidayCache.set(year, result);
  return result;
}

const EVENTS: readonly (readonly [number, number, string])[] = [
  [1, 2, "年の始まり"],
  [2, 14, "バレンタインデー"],
  [3, 3, "ひな祭り"],
  [3, 14, "ホワイトデー"],
  [4, 1, "エイプリルフール"],
  [4, 22, "アースデイ"],
  [5, 11, "母の日"],
  [6, 15, "父の日"],
  [7, 7, "七夕"],
  [8, 13, "お盆"],
  [8, 15, "お盆"],
  [10, 31, "ハロウィン"],
  [11, 11, "ポインセチア"],
  [12, 24, "クリスマスイブ"],
  [12, 25, "クリスマス"],
  [12, 31, "大晦日"],
];

const eventSet = new Map<string, string>();
for (const [month, day, name] of EVENTS) eventSet.set(`${month}-${day}`, name);

function lookup<T>(
  year: number,
  number: number,
  previousYear: Map<number, T>,
  currentYear: Map<number, T>,
): T | null {
  return currentYear.get(number) ?? previousYear.get(number) ?? null;
}

/** その日の祝日・振替休日・国民の休日。 */
export function holidayOf(date: Date): Entry | null {
  const { year, month, day } = calendarDate(date);
  const number = dayNumber(year, month, day);
  return lookup(year, number, holidaysOfYear(year - 1), holidaysOfYear(year));
}

/** その日の季節の行事（祝日と重ならない日のみ）。 */
export function eventOfDay(date: Date): Entry | null {
  const { month, day } = calendarDate(date);
  if (holidayOf(date)) return null;
  const name = eventSet.get(`${month}-${day}`);
  return name ? { name, kind: "event" } : null;
}

/** その日にあたる二十四節気。 */
export function solarTermOf(date: Date): Entry | null {
  const { year, month, day } = calendarDate(date);
  const number = dayNumber(year, month, day);
  for (const name of termsOfYear(year).keys()) {
    const current = termsOfYear(year).get(name);
    const previous = termsOfYear(year - 1).get(name);
    if (current === number || previous === number)
      return { name, kind: "term" };
  }
  return null;
}

/** 次の二十四節気を日数つきで返します。 */
export function upcomingSolarTerm(date: Date): {
  name: string;
  inDays: number;
} {
  const { year, month, day } = calendarDate(date);
  const today = dayNumber(year, month, day);
  const terms = termsOfYear(year);
  let best: { name: string; inDays: number } | null = null;
  for (const [name, number] of terms) {
    if (number <= today) continue;
    const inDays = number - today;
    if (!best || inDays < best.inDays) best = { name, inDays };
  }
  if (best) return best;
  const next = termsOfYear(year + 1);
  const candidates = [...next.entries()].sort((a, b) => a[1] - b[1]);
  return { name: candidates[0][0], inDays: candidates[0][1] - today };
}
