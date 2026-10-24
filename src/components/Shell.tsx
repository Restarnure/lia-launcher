import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AlertTriangle, Boxes, Shirt, CheckCircle2, Compass, Copy, Download, FileText, Home, Info, Minus, Settings as SettingsIcon, Square, Terminal, X, XCircle } from "lucide-react";
import { api, type ContentKind } from "../lib/api";
import { bytes } from "../lib/format";
import { t, type Key } from "../lib/i18n";
import { dismissToast, fail, getState, navigate, refreshInstances, setState, toast, useStore } from "../lib/store";
import { Modal, Progress, SkinHead, stageLabel } from "./ui";
import { UpdatePill } from "./Update";
import logo from "../assets/logo.svg";

export function Logo({ size = 22 }: { size?: number }) {
  return <img src={logo} width={size} height={size} alt="" draggable={false} />;
}

// ------------------------------------------------------------ заголовок вікна

export function TitleBar() {
  const win = getCurrentWindow();
  return (
    <header className="titlebar">
      <div className="brand" data-tauri-drag-region>
        <Logo size={20} />
        <b>ЛІЯ</b>
      </div>
      <div className="drag" data-tauri-drag-region />
      <UpdatePill />
      <TasksButton />
      <button className="win-btn" onClick={() => win.minimize()} aria-label={t("common.minimize")}>
        <Minus size={16} />
      </button>
      <button className="win-btn" onClick={() => win.toggleMaximize()} aria-label={t("common.maximize")}>
        <Square size={13} />
      </button>
      <button className="win-btn close" onClick={() => win.close()} aria-label={t("common.close")}>
        <X size={17} />
      </button>
    </header>
  );
}

function TasksButton() {
  const tasks = useStore((s) => s.tasks);
  const [open, setOpen] = useState(false);
  const list = Object.values(tasks).sort((a, b) => b.id - a.id);
  const active = list.filter((x) => x.state === "running");
  const known = active.filter((x) => x.total > 0);
  const overall = known.length ? known.reduce((a, x) => a + x.done / x.total, 0) / known.length : 0;

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".tasks-panel, .tasks-btn")) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  return (
    <>
      <button className="tasks-btn" onClick={() => setOpen((o) => !o)} aria-label={t("tasks.title")}>
        {active.length > 0 && (
          <svg className="ring" viewBox="0 0 36 36">
            <circle cx="18" cy="18" r="15" fill="none" stroke="rgba(255,255,255,.08)" strokeWidth="3" />
            <circle cx="18" cy="18" r="15" fill="none" stroke="#ffd23f" strokeWidth="3" strokeLinecap="round" strokeDasharray={`${overall * 94.2} 94.2`} />
          </svg>
        )}
        <Download size={17} />
        {active.length > 0 && <span className="num">{active.length}</span>}
      </button>
      {open && (
        <div className="tasks-panel">
          <div className="row" style={{ padding: "4px 10px 8px" }}>
            <h3 className="grow">{t("tasks.title")}</h3>
          </div>
          {list.length === 0 && <div className="empty" style={{ padding: 24 }}>{t("tasks.empty")}</div>}
          {list.map((task) => {
            const pct = task.total > 0 ? (task.done / task.total) * 100 : 0;
            return (
              <div className="task" key={task.id}>
                <div className="top">
                  {task.state === "done" && <CheckCircle2 size={16} color="var(--green)" />}
                  {task.state === "error" && <XCircle size={16} color="var(--red)" />}
                  {task.state === "running" && <div className="spinner" style={{ width: 14, height: 14 }} />}
                  <b className="grow ellipsis">{task.title}</b>
                  {task.state === "running" && (
                    <button className="btn ghost sm icon" onClick={() => api.taskCancel(task.id)} title={t("common.cancel")}>
                      <X size={15} />
                    </button>
                  )}
                </div>
                {task.state === "running" && <Progress value={pct} indeterminate={task.total === 0} />}
                <small>
                  {task.state === "running"
                    ? `${stageLabel(task.stage)}${task.total > 0 ? ` · ${task.unit === "bytes" ? `${bytes(task.done)} / ${bytes(task.total)}` : `${task.done} / ${task.total}`}` : ""}`
                    : task.state === "done"
                      ? t("tasks.done")
                      : task.state === "cancelled"
                        ? t("tasks.cancelled")
                        : t("tasks.failed")}
                </small>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}

// ------------------------------------------------------------ бічна панель

export function Sidebar() {
  const route = useStore((s) => s.route);
  const accounts = useStore((s) => s.accounts);
  const me = accounts.accounts.find((a) => a.id === accounts.selected);
  const items: { page: "home" | "instances" | "browse" | "skins"; icon: React.ReactNode; label: Key }[] = [
    { page: "home", icon: <Home size={21} />, label: "nav.home" },
    { page: "instances", icon: <Boxes size={21} />, label: "nav.instances" },
    { page: "browse", icon: <Compass size={21} />, label: "nav.browse" },
    { page: "skins", icon: <Shirt size={21} />, label: "nav.skins" },
  ];
  const active = route.page === "instance" ? "instances" : route.page;
  return (
    <nav className="sidebar">
      {items.map((it) => (
        <button key={it.page} className={`nav-item ${active === it.page ? "active" : ""}`} onClick={() => navigate({ page: it.page })}>
          {it.icon}
          {t(it.label)}
        </button>
      ))}
      <div className="spacer" />
      <button className={`nav-item ${active === "settings" ? "active" : ""}`} onClick={() => navigate({ page: "settings" })}>
        <SettingsIcon size={21} />
        {t("nav.settings")}
      </button>
      <button className="me" onClick={() => navigate({ page: "accounts" })} title={me ? me.name : t("nav.accounts")}>
        <SkinHead accountId={me?.id} name={me?.name ?? "+"} size={38} />
      </button>
    </nav>
  );
}

// ------------------------------------------------------------ сповіщення

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  return (
    <div className="toasts">
      {toasts.map((x) => (
        <div key={x.id} className={`toast ${x.kind}`} onClick={() => dismissToast(x.id)}>
          <span className="t-icon">
            {x.kind === "error" ? <AlertTriangle size={18} /> : x.kind === "success" ? <CheckCircle2 size={18} /> : <Info size={18} />}
          </span>
          <div className="grow selectable">
            <b>{x.title}</b>
            {x.text && <p>{x.text}</p>}
          </div>
        </div>
      ))}
    </div>
  );
}

// ------------------------------------------------------------ падіння гри

export function CrashDialog() {
  const crash = useStore((s) => s.crash);
  const instances = useStore((s) => s.instances);
  if (!crash) return null;
  const inst = instances.find((i) => i.id === crash.instanceId);
  const close = () => setState({ crash: null });
  const key = `crash.${crash.info.kind}` as Key;
  const text = t(key) === key ? t("crash.unknown") : t(key);
  return (
    <Modal
      title={t("crash.title")}
      icon={<AlertTriangle size={22} color="var(--red)" />}
      onClose={close}
      size="wide"
      foot={
        <>
          {crash.info.reportPath && (
            <button className="btn" onClick={() => api.openPath(crash.info.reportPath!).catch(fail)}>
              <FileText size={16} />
              {t("crash.open_report")}
            </button>
          )}
          <button
            className="btn"
            onClick={() => {
              close();
              navigate({ page: "instance", id: crash.instanceId, tab: "console" });
            }}
          >
            <Terminal size={16} />
            {t("crash.open_console")}
          </button>
          <button className="btn primary" onClick={close}>
            {t("common.close")}
          </button>
        </>
      }
    >
      <div className="row">
        <span className="badge red">{inst?.name ?? crash.instanceId}</span>
        {crash.code !== null && <span className="badge">{t("crash.code", { code: crash.code })}</span>}
      </div>
      <p style={{ fontSize: 15 }}>{text}</p>
      {crash.info.detail && (
        <div className="crash-detail selectable">
          <button className="btn ghost sm icon copy" onClick={() => navigator.clipboard.writeText(crash.info.detail!).then(() => toast("info", t("common.copied")))}>
            <Copy size={15} />
          </button>
          <pre>{crash.info.detail}</pre>
        </div>
      )}
    </Modal>
  );
}

// ------------------------------------------------------------ перетягування файлів у вікно

function kindFor(file: string, tab?: string): ContentKind | "mrpack" | null {
  const f = file.toLowerCase();
  if (f.endsWith(".mrpack")) return "mrpack";
  if (f.endsWith(".jar")) return "mods";
  if (f.endsWith(".zip")) return tab === "shaderpacks" ? "shaderpacks" : "resourcepacks";
  return null;
}

// Бібліотека скінів має сенс лише з акаунтом Microsoft: офлайн-акаунту скін не поставити.
const hasMicrosoft = (s: ReturnType<typeof getState>) => s.accounts.accounts.some((a) => a.kind === "microsoft");

export function DropHandler() {
  const [hover, setHover] = useState<string[] | null>(null);
  const route = useStore((s) => s.route);
  const instances = useStore((s) => s.instances);
  const inst = route.page === "instance" ? instances.find((i) => i.id === route.id) : undefined;
  const ms = useStore(hasMicrosoft);

  useEffect(() => {
    let un: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent(async (e) => {
        const p = e.payload;
        if (p.type === "enter") setHover(p.paths);
        else if (p.type === "leave") setHover(null);
        else if (p.type === "drop") {
          setHover(null);
          const r = getState().route;
          const tab = r.page === "instance" ? r.tab : undefined;
          const mrpacks = p.paths.filter((x) => kindFor(x) === "mrpack");
          for (const m of mrpacks) {
            try {
              const i = await api.importMrpack(m);
              toast("success", t("instances.imported", { name: i.name }));
              navigate({ page: "instance", id: i.id });
            } catch (err) {
              fail(err);
            }
          }
          const pngs = p.paths.filter((x) => x.toLowerCase().endsWith(".png"));
          if (r.page === "skins" && pngs.length && hasMicrosoft(getState())) {
            let n = 0;
            for (const f of pngs) await api.skinAddFile(f).then(() => n++, fail);
            if (n) toast("success", t("skins.added", { n }));
            window.dispatchEvent(new CustomEvent("lia:skins-changed"));
            return;
          }
          if (r.page !== "instance") return;
          const groups: Partial<Record<ContentKind, string[]>> = {};
          for (const f of p.paths) {
            const k = kindFor(f, tab);
            if (k && k !== "mrpack") (groups[k] ??= []).push(f);
          }
          let total = 0;
          for (const [k, files] of Object.entries(groups)) total += await api.contentAdd(r.id, k as ContentKind, files).catch((err) => (fail(err), 0));
          if (total > 0) {
            toast("success", t("inst.added", { n: total }));
            window.dispatchEvent(new CustomEvent("lia:content-changed"));
          }
          refreshInstances();
        }
      })
      .then((f) => (un = f));
    return () => un?.();
  }, []);

  const message = useMemo(() => {
    if (!hover) return null;
    if (hover.some((p) => p.toLowerCase().endsWith(".mrpack"))) return t("drop.mrpack");
    if (route.page === "skins" && ms && hover.some((p) => p.toLowerCase().endsWith(".png"))) return t("drop.skins");
    if (inst) return t("drop.mods", { name: inst.name });
    return t("drop.nowhere");
  }, [hover, inst, route.page, ms]);

  if (!message) return null;
  return (
    <div className="drop-zone">
      <div>
        <Download size={34} color="var(--yellow)" />
        <p style={{ marginTop: 10 }}>{message}</p>
      </div>
    </div>
  );
}
