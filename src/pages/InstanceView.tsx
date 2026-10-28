import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ArrowLeft, Clock, Compass, FilePlus2, FolderOpen, Globe2, ImageIcon, Package, Play, RefreshCw, Search, Trash2, X } from "lucide-react";
import { api, type ContentItem, type ContentKind, type ContentUpdate, type Instance, type Shot, type World } from "../lib/api";
import { ago, bytes, date, duration } from "../lib/format";
import { t, type Key } from "../lib/i18n";
import { back, fail, launch, navigate, toast, useStore, type InstanceTab } from "../lib/store";
import { instanceMenu } from "../components/InstanceCard";
import { confirmDialog, Empty, InstanceIcon, LoaderBadge, Menu, PlayButton, Switch } from "../components/ui";
import { PublishButton } from "../components/Publish";
import { FriendsPlayButton } from "../components/LiaId";
import { Console } from "./Console";
import { InstanceSettings } from "./InstanceSettings";

const TABS: InstanceTab[] = ["mods", "resourcepacks", "shaderpacks", "worlds", "screenshots", "console", "settings"];

export function InstanceView({ id, tab }: { id: string; tab?: InstanceTab }) {
  const inst = useStore((s) => s.instances.find((i) => i.id === id));
  const running = useStore((s) => !!s.running[id]);
  const current: InstanceTab = tab ?? (inst?.loader === "vanilla" ? "worlds" : "mods");

  if (!inst) {
    return (
      <div className="page">
        <Empty title={t("err.instance_not_found")}>
          <button className="btn" onClick={() => navigate({ page: "instances" })}>
            {t("common.back")}
          </button>
        </Empty>
      </div>
    );
  }

  return (
    <div className="page inst-page">
      <header className="inst-head">
        <button className="btn ghost icon" onClick={back} aria-label={t("common.back")}>
          <ArrowLeft size={18} />
        </button>
        <div className="inst-art">
          <InstanceIcon inst={inst} size={72} />
        </div>
        <div className="grow" style={{ minWidth: 0 }}>
          <h1 className="ellipsis">{inst.name}</h1>
          <div className="row" style={{ gap: 6, marginTop: 6, flexWrap: "wrap" }}>
            <span className="badge">Minecraft {inst.mcVersion}</span>
            <LoaderBadge loader={inst.loader} version={inst.loaderVersion ? shortLoader(inst) : undefined} />
            {inst.modpack && (
              <span className="badge blue">
                <Package size={12} />
                {t("inst.modpack", { name: `${inst.modpack.name} ${inst.modpack.versionName}`.trim() })}
              </span>
            )}
            {running && (
              <span className="badge green">
                <span className="dot" style={{ width: 6, height: 6 }} />
                {t("common.playing")}
              </span>
            )}
          </div>
          <div className="inst-stats">
            <span>
              <Clock size={13} />
              {t("inst.play_time")}: {duration(inst.playTime)}
            </span>
            <span>
              {t("inst.last_played")}: {inst.lastPlayed ? ago(inst.lastPlayed) : t("time.never")}
            </span>
          </div>
        </div>
        <div className="inst-actions">
          <button className="btn icon" title={t("common.open_folder")} onClick={() => api.instanceFolder(inst.id).then((p) => api.openPath(p)).catch(fail)}>
            <FolderOpen size={17} />
          </button>
          <PublishButton inst={inst} />
          <Menu items={instanceMenu(inst)} className="btn icon" />
          <FriendsPlayButton inst={inst} />
          <PlayButton inst={inst} />
        </div>
      </header>

      <nav className="tabs">
        {TABS.map((k) => (
          <button key={k} className={`tab ${k === current ? "on" : ""}`} onClick={() => navigate({ page: "instance", id, tab: k })}>
            {t(`inst.tab.${k}` as Key)}
          </button>
        ))}
      </nav>

      {(current === "mods" || current === "resourcepacks" || current === "shaderpacks") && <ContentTab key={current} inst={inst} kind={current} />}
      {current === "worlds" && <WorldsTab inst={inst} />}
      {current === "screenshots" && <ShotsTab inst={inst} />}
      {current === "console" && <Console inst={inst} />}
      {current === "settings" && <InstanceSettings inst={inst} />}
    </div>
  );
}

function shortLoader(inst: Instance) {
  const v = inst.loaderVersion ?? "";
  return inst.loader === "forge" && v.startsWith(`${inst.mcVersion}-`) ? v.slice(inst.mcVersion.length + 1) : v;
}

// ------------------------------------------------------------ моди / ресурспаки / шейдери

function ContentTab({ inst, kind }: { inst: Instance; kind: ContentKind }) {
  const [items, setItems] = useState<ContentItem[] | null>(null);
  const [q, setQ] = useState("");
  const [updates, setUpdates] = useState<ContentUpdate[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    try {
      const list = await api.content(inst.id, kind);
      setItems(list);
      if (list.some((i) => !i.projectId && !i.isDir)) {
        api.contentIdentify(inst.id, kind).then(setItems).catch(() => {});
      }
    } catch (e) {
      fail(e);
      setItems([]);
    }
  }, [inst.id, kind]);

  useEffect(() => {
    load();
    const onChange = () => load();
    window.addEventListener("lia:content-changed", onChange);
    return () => window.removeEventListener("lia:content-changed", onChange);
  }, [load]);

  const shown = useMemo(() => {
    const s = q.trim().toLowerCase();
    return (items ?? []).filter((i) => !s || `${i.name} ${i.file}`.toLowerCase().includes(s));
  }, [items, q]);

  const updateOf = (file: string) => updates?.find((u) => u.file === file);

  const check = async () => {
    setChecking(true);
    try {
      const u = await api.contentUpdates(inst.id, kind);
      setUpdates(u);
      toast("info", u.length ? t("inst.updates_found", { n: u.length }) : t("inst.updates_none"));
    } catch (e) {
      fail(e);
    } finally {
      setChecking(false);
    }
  };

  const doUpdate = async (list: ContentUpdate[]) => {
    setUpdating((s) => new Set([...s, ...list.map((u) => u.file)]));
    for (const u of list) {
      try {
        await api.contentUpdate(inst.id, kind, u);
        setUpdates((cur) => cur?.filter((x) => x.file !== u.file) ?? null);
      } catch (e) {
        fail(e);
      }
    }
    // Лише свої файли: оновлення іншого мода, запущене паралельно, ще триває.
    setUpdating((s) => new Set([...s].filter((f) => !list.some((u) => u.file === f))));
    load();
  };

  const toggle = async (item: ContentItem) => {
    try {
      const name = await api.contentToggle(inst.id, kind, item.file);
      setItems((cur) => cur?.map((i) => (i.file === item.file ? { ...i, file: name, enabled: !i.enabled } : i)) ?? null);
    } catch (e) {
      fail(e);
    }
  };

  const remove = async (item: ContentItem) => {
    if (!(await confirmDialog({ title: t("inst.delete_item", { name: item.name }), danger: true }))) return;
    try {
      await api.contentRemove(inst.id, kind, item.file);
      setItems((cur) => cur?.filter((i) => i.file !== item.file) ?? null);
    } catch (e) {
      fail(e);
    }
  };

  const addFile = async () => {
    const files = await open({ multiple: true, filters: [{ name: t(`inst.tab.${kind}` as Key), extensions: kind === "mods" ? ["jar"] : ["zip"] }] });
    if (!files || (Array.isArray(files) && files.length === 0)) return;
    try {
      const n = await api.contentAdd(inst.id, kind, Array.isArray(files) ? files : [files]);
      toast("success", t("inst.added", { n }));
      load();
    } catch (e) {
      fail(e);
    }
  };

  const browseType = kind === "mods" ? "mod" : kind === "resourcepacks" ? "resourcepack" : "shader";
  const vanillaMods = kind === "mods" && inst.loader === "vanilla";

  return (
    <div>
      <div className="toolbar">
        <div className="input-wrap" style={{ width: 260 }}>
          <Search size={16} />
          <input className="input" placeholder={t("inst.filter")} value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
        <div className="grow" />
        {(items?.length ?? 0) > 0 && (
          <button className="btn" onClick={check} disabled={checking}>
            <RefreshCw size={15} className={checking ? "spin" : ""} />
            {checking ? t("inst.checking") : t("inst.check_updates")}
          </button>
        )}
        {updates && updates.length > 0 && (
          <button className="btn blue" onClick={() => doUpdate(updates)} disabled={updating.size > 0}>
            {t("inst.update_all", { n: updates.length })}
          </button>
        )}
        <button className="btn" onClick={addFile} disabled={vanillaMods}>
          <FilePlus2 size={15} />
          {t("inst.add_file")}
        </button>
        <button className="btn primary" disabled={vanillaMods} onClick={() => navigate({ page: "browse", type: browseType, instanceId: inst.id })}>
          <Compass size={15} />
          {t("inst.add_from_modrinth")}
        </button>
      </div>

      {items === null ? (
        <div className="list">
          {[0, 1, 2, 3].map((i) => (
            <div key={i} className="skeleton" style={{ height: 58, margin: 6 }} />
          ))}
        </div>
      ) : items.length === 0 ? (
        vanillaMods ? (
          <Empty icon={<Package size={38} className="faint" />} title={t("inst.empty_vanilla")} text={t("inst.empty_vanilla_text")}>
            <button className="btn" onClick={() => navigate({ page: "instance", id: inst.id, tab: "settings" })}>
              {t("inst.tab.settings")}
            </button>
          </Empty>
        ) : (
          <Empty icon={<Package size={38} className="faint" />} title={t(`inst.empty_${kind}` as Key)} text={t(`inst.empty_${kind}_text` as Key)} />
        )
      ) : (
        <div className="list">
          {shown.map((item) => {
            const up = updateOf(item.file);
            const busy = updating.has(item.file);
            return (
              <div key={item.file} className={`list-row ${item.enabled ? "" : "off"}`}>
                <div className="thumb">{item.iconUrl ? <img src={item.iconUrl} alt="" /> : <Package size={20} className="faint" />}</div>
                <div className="grow" style={{ minWidth: 0 }}>
                  <div className="row" style={{ gap: 8 }}>
                    <b className="ellipsis">{item.name}</b>
                    {item.version && <span className="faint ellipsis" style={{ fontSize: 12.5, maxWidth: 220 }}>{item.version}</span>}
                    {!item.enabled && <span className="badge">{t("inst.disabled")}</span>}
                  </div>
                  <small className="faint ellipsis" style={{ display: "block" }}>
                    {item.file}
                    {item.size > 0 ? ` · ${bytes(item.size)}` : ""}
                  </small>
                </div>
                {up && (
                  <button className="btn sm blue" disabled={busy} onClick={() => doUpdate([up])} title={up.versionNumber}>
                    {busy ? <div className="spinner" style={{ width: 13, height: 13 }} /> : <RefreshCw size={13} />}
                    {t("inst.update")}
                  </button>
                )}
                {item.projectId && (
                  <button className="btn ghost sm icon" title={t("browse.open_site")} onClick={() => api.openUrl(`https://modrinth.com/project/${item.projectId}`).catch(fail)}>
                    <Globe2 size={15} />
                  </button>
                )}
                <Switch on={item.enabled} onChange={() => toggle(item)} label={item.name} />
                <button className="btn ghost sm icon" onClick={() => remove(item)} title={t("common.delete")}>
                  <Trash2 size={15} />
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------ світи

function WorldsTab({ inst }: { inst: Instance }) {
  const [worlds, setWorlds] = useState<World[] | null>(null);
  const load = useCallback(() => api.worlds(inst.id).then(setWorlds).catch((e) => (fail(e), setWorlds([]))), [inst.id]);
  useEffect(() => {
    load();
  }, [load]);
  // Швидкий вхід у світ — з 1.20 (аргумент --quickPlaySingleplayer).
  const quick = useMemo(() => {
    const [a, b] = inst.mcVersion.split(".").map(Number);
    return a >= 2 || (a === 1 && b >= 20);
  }, [inst.mcVersion]);

  if (worlds === null) return <div className="skeleton" style={{ height: 120 }} />;
  if (worlds.length === 0) return <Empty icon={<Globe2 size={38} className="faint" />} title={t("inst.empty_worlds")} text={t("inst.empty_worlds_text")} />;
  return (
    <div className="world-grid">
      {worlds.map((w) => (
        <div key={w.folder} className="world">
          <div className="world-icon">{w.icon ? <img src={convertFileSrc(w.icon)} alt="" /> : <Globe2 size={30} className="faint" />}</div>
          <div className="grow" style={{ minWidth: 0 }}>
            <b className="ellipsis" style={{ display: "block" }}>
              {w.folder}
            </b>
            <small className="faint">
              {date(w.modified)} · {bytes(w.size)}
            </small>
          </div>
          {quick && (
            <button className="btn sm icon" title={t("inst.world_play")} onClick={() => launchWorld(inst, w.folder)}>
              <Play size={14} />
            </button>
          )}
          <button className="btn ghost sm icon" title={t("common.open_folder")} onClick={() => api.instanceFolder(inst.id, `saves/${w.folder}`).then((p) => api.openPath(p)).catch(fail)}>
            <FolderOpen size={15} />
          </button>
          <button
            className="btn ghost sm icon"
            title={t("common.delete")}
            onClick={async () => {
              if (!(await confirmDialog({ title: t("inst.delete_world", { name: w.folder }), text: t("inst.delete_world_text"), danger: true }))) return;
              api.trashGameFile(inst.id, `saves/${w.folder}`).then(load).catch(fail);
            }}
          >
            <Trash2 size={15} />
          </button>
        </div>
      ))}
    </div>
  );
}

function launchWorld(inst: Instance, folder: string) {
  launch(inst.id, undefined, folder);
}

// ------------------------------------------------------------ скріншоти

function ShotsTab({ inst }: { inst: Instance }) {
  const [shots, setShots] = useState<Shot[] | null>(null);
  const [view, setView] = useState<Shot | null>(null);
  const load = useCallback(() => api.screenshots(inst.id).then(setShots).catch((e) => (fail(e), setShots([]))), [inst.id]);
  useEffect(() => {
    load();
  }, [load]);
  useEffect(() => {
    if (!view) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setView(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view]);
  if (shots === null) return <div className="skeleton" style={{ height: 160 }} />;
  if (shots.length === 0) return <Empty icon={<ImageIcon size={38} className="faint" />} title={t("inst.empty_shots")} text={t("inst.empty_shots_text")} />;
  return (
    <>
      <div className="toolbar">
        <div className="grow" />
        <button className="btn" onClick={() => api.instanceFolder(inst.id, "screenshots").then((p) => api.openPath(p)).catch(fail)}>
          <FolderOpen size={15} />
          {t("common.open_folder")}
        </button>
      </div>
      <div className="shot-grid">
        {shots.map((s) => (
          <button key={s.path} className="shot" onClick={() => setView(s)}>
            <img src={convertFileSrc(s.path)} alt={s.name} loading="lazy" />
          </button>
        ))}
      </div>
      {view && (
        <div className="lightbox" onClick={() => setView(null)}>
          <img src={convertFileSrc(view.path)} alt="" />
          <div className="lightbox-bar" onClick={(e) => e.stopPropagation()}>
            <span className="grow ellipsis">{view.name}</span>
            <button className="btn sm" onClick={() => api.openPath(view.path, true).catch(fail)}>
              <FolderOpen size={14} />
            </button>
            <button
              className="btn sm danger"
              onClick={() => {
                api
                  .trashGameFile(inst.id, `screenshots/${view.name}`)
                  .then(() => {
                    setView(null);
                    load();
                  })
                  .catch(fail);
              }}
            >
              <Trash2 size={14} />
            </button>
            <button className="btn sm" onClick={() => setView(null)}>
              <X size={14} />
            </button>
          </div>
        </div>
      )}
    </>
  );
}
