import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Copy, FolderOpen, Search, Square } from "lucide-react";
import { api, type Instance, type LogLine } from "../lib/api";
import { clock } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, kill, setState, toast, useStore } from "../lib/store";
import { Empty, Seg, Switch } from "../components/ui";

const ROW = 20;
const EMPTY: LogLine[] = [];

type Level = "all" | "warn" | "error";

interface Row {
  t: number;
  level: string;
  thread: string;
  text: string;
  first: boolean;
}

function passes(l: LogLine, level: Level) {
  if (level === "all") return true;
  const v = l.level.toUpperCase();
  if (level === "error") return v === "ERROR" || v === "FATAL";
  return v === "WARN" || v === "ERROR" || v === "FATAL";
}

export function Console({ inst }: { inst: Instance }) {
  const lines = useStore((s) => s.logs[inst.id] ?? EMPTY);
  const running = useStore((s) => !!s.running[inst.id]);
  const [level, setLevel] = useState<Level>("all");
  const [q, setQ] = useState("");
  const [follow, setFollow] = useState(true);
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(400);
  const view = useRef<HTMLDivElement>(null);

  // Після перезапуску лаунчера чи переходу між сторінками — лог останньої сесії з ядра.
  useEffect(() => {
    if (lines.length > 0) return;
    api
      .gameLog(inst.id)
      .then((l) => l.length && setState((s) => ({ logs: { ...s.logs, [inst.id]: l } })))
      .catch(() => {});
  }, [inst.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const rows = useMemo(() => {
    const s = q.trim().toLowerCase();
    const out: Row[] = [];
    for (const l of lines) {
      if (!passes(l, level)) continue;
      if (s && !l.text.toLowerCase().includes(s) && !l.thread.toLowerCase().includes(s)) continue;
      l.text.split("\n").forEach((text, i) => out.push({ t: l.t, level: l.level.toUpperCase(), thread: l.thread, text, first: i === 0 }));
    }
    return out;
  }, [lines, level, q]);

  useLayoutEffect(() => {
    const el = view.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (follow && view.current) view.current.scrollTop = view.current.scrollHeight;
  }, [rows.length, follow]);

  const onScroll = () => {
    const el = view.current!;
    setScroll(el.scrollTop);
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < ROW * 2;
    if (atBottom !== follow) setFollow(atBottom);
  };

  const start = Math.max(0, Math.floor(scroll / ROW) - 20);
  const end = Math.min(rows.length, Math.ceil((scroll + height) / ROW) + 20);

  const copy = () => {
    const text = lines.map((l) => `[${clock(l.t)}] [${l.thread}/${l.level}] ${l.text}`).join("\n");
    navigator.clipboard.writeText(text).then(() => toast("info", t("common.copied")));
  };

  return (
    <div className="console">
      <div className="toolbar">
        <Seg<Level>
          value={level}
          onChange={setLevel}
          options={[
            { value: "all", label: t("console.all") },
            { value: "warn", label: t("console.warn") },
            { value: "error", label: t("console.error") },
          ]}
        />
        <div className="input-wrap" style={{ width: 240 }}>
          <Search size={16} />
          <input className="input" placeholder={t("console.search")} value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
        <div className="grow" />
        <span className="faint" style={{ fontSize: 12.5 }}>
          {t("console.lines", { n: lines.length })}
        </span>
        <label className="row" style={{ gap: 8, fontSize: 13 }}>
          <Switch on={follow} onChange={setFollow} label={t("console.follow")} />
          {t("console.follow")}
        </label>
        <button className="btn icon" title={t("console.copy")} onClick={copy} disabled={!lines.length}>
          <Copy size={16} />
        </button>
        <button className="btn icon" title={t("console.logs")} onClick={() => api.instanceFolder(inst.id, "logs").then((p) => api.openPath(p)).catch(fail)}>
          <FolderOpen size={16} />
        </button>
        {running && (
          <button className="btn danger" onClick={() => kill(inst.id)}>
            <Square size={13} fill="currentColor" />
            {t("common.stop")}
          </button>
        )}
      </div>
      <div className="console-view selectable" ref={view} onScroll={onScroll}>
        {rows.length === 0 ? (
          <Empty title={t("console.empty")} />
        ) : (
          <div style={{ height: rows.length * ROW, position: "relative" }}>
            {rows.slice(start, end).map((r, i) => (
              <div key={start + i} className={`ln lvl-${r.level || "OUT"} ${r.first ? "" : "cont"}`} style={{ top: (start + i) * ROW }}>
                {r.first ? (
                  <>
                    <span className="ts">{clock(r.t)}</span>
                    <span className="th">{r.thread || r.level || "out"}</span>
                  </>
                ) : (
                  <span className="ts" />
                )}
                <span className="tx">{r.text}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
