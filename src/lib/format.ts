import { getLang, t } from "./i18n";

export function bytes(n: number): string {
  const uk = getLang() === "uk";
  const units = uk ? ["Б", "КБ", "МБ", "ГБ"] : ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i === 0 || n >= 100 ? 0 : 1)} ${units[i]}`;
}

export function compact(n: number): string {
  const uk = getLang() === "uk";
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}${uk ? " млн" : "M"}`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}${uk ? " тис." : "K"}`;
  return String(n);
}

export function ago(ms: number | null | undefined): string {
  if (!ms) return t("time.never");
  const diff = Date.now() - ms;
  const min = Math.floor(diff / 60000);
  if (min < 1) return t("time.just_now");
  if (min < 60) return t("time.minutes_ago", { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t("time.hours_ago", { n: h });
  const d = Math.floor(h / 24);
  if (d < 30) return t("time.days_ago", { n: d });
  return date(ms);
}

export function date(ms: number | string): string {
  return new Date(ms).toLocaleDateString(getLang() === "uk" ? "uk-UA" : "en-GB", { day: "numeric", month: "long", year: "numeric" });
}

export function duration(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return h > 0 ? t("time.hm", { h, m }) : t("time.m", { m });
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(getLang() === "uk" ? "uk-UA" : "en-GB", { hour12: false });
}

export function gb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} ${getLang() === "uk" ? "ГБ" : "GB"}`;
}
