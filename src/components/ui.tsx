import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Check as CheckIcon, ChevronDown, MoreHorizontal, Play, Search, Square, X } from "lucide-react";
import type { Instance, Loader, SkinInfo } from "../lib/api";
import { api } from "../lib/api";
import { blockImage } from "../lib/blocks";
import { t, type Key } from "../lib/i18n";
import { getState, kill, launch, useInstanceTask, useStore } from "../lib/store";

// ------------------------------------------------------------ модальне вікно

export function Modal(props: { title?: ReactNode; onClose: () => void; children: ReactNode; foot?: ReactNode; size?: "wide" | "xwide"; icon?: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [props.onClose]);
  return createPortal(
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className={`modal ${props.size ?? ""}`} role="dialog" aria-modal="true">
        {props.title !== undefined && (
          <div className="modal-head">
            {props.icon}
            <h2>{props.title}</h2>
            <button className="btn ghost icon sm" onClick={props.onClose} aria-label={t("common.close")}>
              <X size={18} />
            </button>
          </div>
        )}
        <div className="modal-body">{props.children}</div>
        {props.foot && <div className="modal-foot">{props.foot}</div>}
      </div>
    </div>,
    document.body,
  );
}

// ------------------------------------------------------------ підтвердження

interface ConfirmReq {
  title: string;
  text?: string;
  ok?: string;
  danger?: boolean;
  resolve: (v: boolean) => void;
}

let showConfirm: ((r: ConfirmReq | null) => void) | null = null;

export function confirmDialog(o: Omit<ConfirmReq, "resolve">): Promise<boolean> {
  return new Promise((resolve) => showConfirm?.({ ...o, resolve }));
}

export function ConfirmHost() {
  const [req, setReq] = useState<ConfirmReq | null>(null);
  useEffect(() => {
    showConfirm = setReq;
    return () => {
      showConfirm = null;
    };
  }, []);
  if (!req) return null;
  const done = (v: boolean) => {
    req.resolve(v);
    setReq(null);
  };
  return (
    <Modal
      title={req.title}
      onClose={() => done(false)}
      foot={
        <>
          <button className="btn ghost" onClick={() => done(false)}>
            {t("common.cancel")}
          </button>
          <button className={`btn ${req.danger ? "danger" : "primary"}`} onClick={() => done(true)} autoFocus>
            {req.ok ?? t("common.delete")}
          </button>
        </>
      }
    >
      {req.text && <p className="muted">{req.text}</p>}
    </Modal>
  );
}

// ------------------------------------------------------------ спливаючий список

function usePopover(open: boolean, anchor: React.RefObject<HTMLElement | null>, close: () => void, minWidth?: number) {
  const [pos, setPos] = useState<{ top: number; left: number; width: number; maxHeight: number } | null>(null);
  const pop = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!open || !anchor.current) return;
    const r = anchor.current.getBoundingClientRect();
    const width = Math.max(r.width, minWidth ?? 0);
    const below = window.innerHeight - r.bottom - 12;
    const above = r.top - 54;
    const maxHeight = Math.min(340, Math.max(below, above));
    const top = below >= 200 || below >= above ? r.bottom + 6 : r.top - 6 - Math.min(340, above);
    const left = Math.min(r.left, window.innerWidth - width - 10);
    setPos({ top, left, width, maxHeight });
  }, [open, anchor, minWidth]);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!pop.current?.contains(e.target as Node) && !anchor.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, close, anchor]);
  return { pos, pop };
}

export interface Option<T extends string = string> {
  value: T;
  label: ReactNode;
  sub?: ReactNode;
  icon?: ReactNode;
  search?: string;
}

export function Select<T extends string>(props: {
  value: T | null;
  options: Option<T>[];
  onChange: (v: T) => void;
  placeholder?: string;
  searchable?: boolean;
  disabled?: boolean;
  minWidth?: number;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [hl, setHl] = useState(0);
  const btn = useRef<HTMLButtonElement>(null);
  const close = useMemo(() => () => setOpen(false), []);
  const { pos, pop } = usePopover(open, btn, close, props.minWidth);
  const current = props.options.find((o) => o.value === props.value);
  const shown = useMemo(() => {
    const s = q.trim().toLowerCase();
    return s ? props.options.filter((o) => (o.search ?? String(o.value)).toLowerCase().includes(s)) : props.options;
  }, [q, props.options]);

  useEffect(() => {
    if (open) {
      setQ("");
      setHl(Math.max(0, props.options.findIndex((o) => o.value === props.value)));
    }
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!open) return;
    pop.current?.querySelector(".option.hl")?.scrollIntoView({ block: "nearest" });
  }, [hl, open, pop]);

  const pick = (v: T) => {
    props.onChange(v);
    setOpen(false);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHl((h) => Math.min(shown.length - 1, h + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHl((h) => Math.max(0, h - 1));
    } else if (e.key === "Enter" && shown[hl]) {
      e.preventDefault();
      pick(shown[hl].value);
    }
  };

  return (
    <div className={`select ${props.className ?? ""}`}>
      <button ref={btn} className={`select-btn ${open ? "open" : ""}`} disabled={props.disabled} onClick={() => setOpen((o) => !o)} onKeyDown={open ? onKey : undefined}>
        {current?.icon}
        <span className="grow ellipsis">{current ? current.label : <span className="faint">{props.placeholder}</span>}</span>
        {current?.sub && <span className="faint" style={{ fontSize: 12 }}>{current.sub}</span>}
        <ChevronDown size={16} className="faint" />
      </button>
      {open &&
        pos &&
        createPortal(
          <div ref={pop} className="popover" style={{ top: pos.top, left: pos.left, width: pos.width, maxHeight: pos.maxHeight }} onKeyDown={onKey}>
            {props.searchable && (
              <div className="search">
                <div className="input-wrap">
                  <Search size={15} />
                  <input
                    className="input"
                    style={{ height: 34 }}
                    autoFocus
                    value={q}
                    placeholder={t("new.search_version")}
                    onChange={(e) => {
                      setQ(e.target.value);
                      setHl(0);
                    }}
                  />
                </div>
              </div>
            )}
            {shown.map((o, i) => (
              <button key={o.value} className={`option ${o.value === props.value ? "on" : ""} ${i === hl ? "hl" : ""}`} onMouseEnter={() => setHl(i)} onClick={() => pick(o.value)}>
                {o.icon}
                <span className="ellipsis">{o.label}</span>
                {o.sub && <span className="sub">{o.sub}</span>}
                {o.value === props.value && !o.sub && <CheckIcon size={15} style={{ marginLeft: "auto" }} />}
              </button>
            ))}
            {shown.length === 0 && <div className="option faint">{t("instances.no_match")}</div>}
          </div>,
          document.body,
        )}
    </div>
  );
}

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  onClick: () => void;
  danger?: boolean;
  separator?: boolean;
}

export function Menu(props: { items: MenuItem[]; trigger?: ReactNode; className?: string; label?: string }) {
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const close = useMemo(() => () => setOpen(false), []);
  const { pos, pop } = usePopover(open, btn, close, 210);
  return (
    <>
      <button
        ref={btn}
        className={props.className ?? "btn ghost icon sm"}
        aria-label={props.label ?? "menu"}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
      >
        {props.trigger ?? <MoreHorizontal size={18} />}
      </button>
      {open &&
        pos &&
        createPortal(
          <div ref={pop} className="popover" style={{ top: pos.top, left: Math.max(10, pos.left - (210 - (btn.current?.offsetWidth ?? 0))), width: 210 }}>
            {props.items.map((it, i) => (
              <div key={i}>
                {it.separator && <div className="menu-sep" />}
                <button
                  className={`option ${it.danger ? "danger" : ""}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setOpen(false);
                    it.onClick();
                  }}
                >
                  {it.icon}
                  {it.label}
                </button>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}

// ------------------------------------------------------------ дрібні елементи

export function Switch(props: { on: boolean; onChange: (v: boolean) => void; label?: string }) {
  return <button className={`switch ${props.on ? "on" : ""}`} role="switch" aria-checked={props.on} aria-label={props.label} onClick={() => props.onChange(!props.on)} />;
}

export function Check(props: { on: boolean; onChange: (v: boolean) => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button className={`check ${props.on ? "on" : ""}`} disabled={props.disabled} role="checkbox" aria-checked={props.on} onClick={() => props.onChange(!props.on)}>
      <span className="box">{props.on && <CheckIcon size={12} strokeWidth={3.5} />}</span>
      {props.children}
    </button>
  );
}

export function Seg<T extends string>(props: { value: T; options: { value: T; label: ReactNode }[]; onChange: (v: T) => void }) {
  return (
    <div className="seg">
      {props.options.map((o) => (
        <button key={o.value} className={o.value === props.value ? "on" : ""} onClick={() => props.onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Slider(props: { value: number; min: number; max: number; step?: number; onChange: (v: number) => void }) {
  const p = ((props.value - props.min) / Math.max(1, props.max - props.min)) * 100;
  return (
    <input
      type="range"
      className="slider"
      min={props.min}
      max={props.max}
      step={props.step ?? 1}
      value={props.value}
      style={{ ["--p" as string]: `${p}%` }}
      onChange={(e) => props.onChange(Number(e.target.value))}
    />
  );
}

export function Progress({ value, indeterminate }: { value: number; indeterminate?: boolean }) {
  return (
    <div className={`progress ${indeterminate ? "indeterminate" : ""}`}>
      <div style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </div>
  );
}

export function Empty(props: { icon?: ReactNode; title: string; text?: string; children?: ReactNode }) {
  return (
    <div className="empty">
      {props.icon}
      <h3>{props.title}</h3>
      {props.text && <p>{props.text}</p>}
      {props.children}
    </div>
  );
}

const LOADER_NAMES: Record<Loader, string> = { vanilla: "Vanilla", fabric: "Fabric", quilt: "Quilt", forge: "Forge", neoforge: "NeoForge" };

export function loaderName(l: Loader) {
  return LOADER_NAMES[l] ?? l;
}

export function LoaderBadge({ loader, version }: { loader: Loader; version?: string }) {
  return <span className={`badge ${loader}`}>{version ? `${loaderName(loader)} ${version}` : loaderName(loader)}</span>;
}

/** Шлях до файлу в теці даних для показу через asset://. */
export function dataFile(...parts: string[]): string {
  const root = getState().info?.dataDir ?? "";
  const sep = root.includes("\\") ? "\\" : "/";
  return convertFileSrc([root, ...parts].join(sep));
}

export function InstanceIcon({ inst, size = 48 }: { inst: Pick<Instance, "id" | "icon">; size?: number }) {
  const [broken, setBroken] = useState(false);
  let src: string;
  if (inst.icon.startsWith("custom") && !broken) src = `${dataFile("instances", inst.id, "icon.png")}?v=${inst.icon.split(":")[1] ?? ""}`;
  else if (inst.icon === "featured:shotam") src = blockImage("shotam", 128);
  else src = blockImage(inst.icon.startsWith("block:") ? inst.icon.slice(6) : "grass", 128);
  const custom = inst.icon.startsWith("custom") && !broken;
  return (
    <img
      src={src}
      width={size}
      height={size}
      alt=""
      draggable={false}
      onError={() => setBroken(true)}
      style={{ borderRadius: custom ? size / 5 : 0, objectFit: "cover", imageRendering: custom ? "auto" : "pixelated", flex: "none" }}
    />
  );
}

// ------------------------------------------------------------ голова скіна

const skinCache = new Map<string, Promise<SkinInfo>>();

export function loadSkin(accountId: string, fresh = false): Promise<SkinInfo> {
  if (fresh || !skinCache.has(accountId)) {
    skinCache.set(
      accountId,
      api.skin(accountId).catch(() => ({ skin: null, slim: false, capes: [], source: "default" as const })),
    );
  }
  return skinCache.get(accountId)!;
}

const headCache = new Map<string, string>();

function headFrom(skin: string): Promise<string> {
  const hit = headCache.get(skin);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = c.height = 64;
      const g = c.getContext("2d")!;
      g.imageSmoothingEnabled = false;
      g.drawImage(img, 8, 8, 8, 8, 4, 4, 56, 56);
      g.drawImage(img, 40, 8, 8, 8, 0, 0, 64, 64);
      const url = c.toDataURL();
      headCache.set(skin, url);
      resolve(url);
    };
    img.onerror = () => resolve("");
    img.src = skin;
  });
}

export function SkinHead({ accountId, name, size = 36 }: { accountId?: string | null; name?: string; size?: number }) {
  const [src, setSrc] = useState("");
  useEffect(() => {
    let live = true;
    setSrc("");
    if (accountId)
      loadSkin(accountId).then((s) => {
        if (s.skin) headFrom(s.skin).then((u) => live && setSrc(u));
      });
    return () => {
      live = false;
    };
  }, [accountId]);
  if (src) return <img src={src} width={size} height={size} alt="" style={{ imageRendering: "pixelated", borderRadius: size / 5, flex: "none" }} />;
  return (
    <div
      style={{
        width: size,
        height: size,
        borderRadius: size / 5,
        flex: "none",
        display: "grid",
        placeItems: "center",
        background: "linear-gradient(135deg, #2f66e0, #1b3f99)",
        fontFamily: "var(--font-display)",
        fontWeight: 700,
        fontSize: size * 0.42,
        color: "#ffd23f",
      }}
    >
      {(name ?? "?").slice(0, 1).toUpperCase()}
    </div>
  );
}

// ------------------------------------------------------------ кнопка «Грати»

export function stageLabel(stage: string): string {
  const key = `stage.${stage}` as Key;
  const s = t(key);
  return s === key ? t("common.preparing") : s;
}

export function PlayButton({ inst, size, server, label }: { inst: Instance; size?: "sm"; server?: { host: string; port?: number }; label?: string }) {
  const running = useStore((s) => s.running[inst.id]);
  const task = useInstanceTask(inst.id);
  if (task) {
    const pct = task.total > 0 ? Math.floor((task.done / task.total) * 100) : 0;
    return (
      <button className={`play busy ${size ?? ""}`} onClick={() => api.taskCancel(task.id)} title={t("common.cancel")}>
        <div className={`fill ${task.total > 0 ? "" : "indeterminate"}`} style={{ width: `${pct}%` }} />
        <span className="ellipsis">{task.total > 0 ? `${stageLabel(task.stage)} · ${pct}%` : stageLabel(task.stage)}</span>
      </button>
    );
  }
  if (running) {
    return (
      <button className={`play running ${size ?? ""}`} onClick={() => kill(inst.id)} title={t("common.stop")}>
        <Square size={size ? 14 : 16} fill="currentColor" />
        <span>{t("common.playing")}</span>
      </button>
    );
  }
  return (
    <button className={`play ${size ?? ""}`} onClick={() => launch(inst.id, server)}>
      <Play size={size ? 15 : 18} fill="currentColor" />
      <span>{(label ?? t("common.play")).toUpperCase()}</span>
    </button>
  );
}
