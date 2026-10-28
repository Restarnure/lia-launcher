import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { SkinViewer, WalkingAnimation } from "skinview3d";
import { Brush, Check, FileUp, Pencil, Plus, Save, Search, Trash2, UserRound } from "lucide-react";
import { api, type Account, type LibSkin, type SkinInfo, type SkinsState } from "../lib/api";
import { t } from "../lib/i18n";
import { skinThumb } from "../lib/skinrender";
import { fail, navigate, refreshAccounts, setState, toast, useStore } from "../lib/store";
import { confirmDialog, Empty, loadSkin, Modal, Seg, Select, SkinHead } from "../components/ui";

type Pick = { key: string; libId?: string; defaultId?: string; name: string; data: string; slim: boolean; source: string };

/** Мініатюра скіна в бібліотеці: 3D-знімок, поки він рендериться — сама текстура. */
function SkinCard(props: { pick: Pick; active: boolean; applied: boolean; onClick: () => void; actions?: React.ReactNode }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    skinThumb(props.pick.key, props.pick.data, props.pick.slim)
      .then((u) => live && setSrc(u))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [props.pick.key, props.pick.data, props.pick.slim]);
  return (
    <div className={`skin-card ${props.active ? "on" : ""}`} onClick={props.onClick}>
      <div className="skin-thumb">{src ? <img src={src} alt="" draggable={false} /> : <div className="spinner" />}</div>
      <div className="skin-card-name ellipsis">{props.pick.name}</div>
      {props.applied && (
        <span className="skin-applied">
          <Check size={11} strokeWidth={3} />
          {t("skins.applied")}
        </span>
      )}
      {props.actions && (
        <div className="skin-card-actions" onClick={(e) => e.stopPropagation()}>
          {props.actions}
        </div>
      )}
    </div>
  );
}

/** Великий 3D-перегляд: обертається мишею, ходьба. */
function Stage({ data, slim, cape }: { data: string | null; slim: boolean; cape: string | null }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<SkinViewer | null>(null);
  useEffect(() => {
    const v = new SkinViewer({ canvas: canvas.current!, width: 300, height: 400, fov: 50, zoom: 0.8 });
    v.autoRotate = true;
    v.autoRotateSpeed = 0.5;
    v.controls.enableZoom = false;
    const anim = new WalkingAnimation();
    anim.speed = 0.5;
    v.animation = anim;
    viewer.current = v;
    return () => {
      v.dispose();
      v.renderer.forceContextLoss();
    };
  }, []);
  useEffect(() => {
    const v = viewer.current;
    if (!v) return;
    if (data) v.loadSkin(data, { model: slim ? "slim" : "default" });
    else v.loadSkin(null);
  }, [data, slim]);
  useEffect(() => {
    const v = viewer.current;
    if (!v) return;
    if (cape) v.loadCape(cape);
    else v.loadCape(null);
  }, [cape]);
  return (
    <div className="skin-stage big">
      <canvas ref={canvas} />
    </div>
  );
}

function NickModal({ onClose, onAdded }: { onClose: () => void; onAdded: (s: LibSkin) => void }) {
  const [nick, setNick] = useState("");
  const [busy, setBusy] = useState(false);
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(nick);
  const go = async () => {
    setBusy(true);
    try {
      onAdded(await api.skinAddNick(nick));
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={t("skins.by_nick")}
      onClose={onClose}
      foot={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={!valid || busy} onClick={go}>
            {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {t("skins.add")}
          </button>
        </>
      }
    >
      <p className="muted">{t("skins.by_nick_text")}</p>
      <input
        className="input"
        autoFocus
        placeholder="Notch"
        value={nick}
        maxLength={16}
        onChange={(e) => setNick(e.target.value.replace(/[^A-Za-z0-9_]/g, ""))}
        onKeyDown={(e) => e.key === "Enter" && valid && go()}
      />
    </Modal>
  );
}

export function Skins({ initialAccount }: { initialAccount?: string }) {
  const { accounts, selected } = useStore((s) => s.accounts);
  const [accountId, setAccountId] = useState<string | null>(initialAccount ?? selected);
  const account: Account | undefined = accounts.find((a) => a.id === accountId) ?? accounts.find((a) => a.id === selected) ?? accounts[0];
  const [lib, setLib] = useState<SkinsState | null>(null);
  const [current, setCurrent] = useState<SkinInfo | null>(null);
  const [pick, setPick] = useState<Pick | null>(null);
  const [slim, setSlim] = useState(false);
  const [cape, setCape] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [nickOpen, setNickOpen] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const ms = account?.kind === "microsoft";

  const reload = useCallback(async () => {
    try {
      setLib(await api.skins());
    } catch (e) {
      fail(e);
    }
  }, []);

  useEffect(() => {
    reload();
    const onDrop = () => reload();
    window.addEventListener("lia:skins-changed", onDrop);
    return () => window.removeEventListener("lia:skins-changed", onDrop);
  }, [reload]);

  useEffect(() => {
    setCurrent(null);
    if (!account) return;
    loadSkin(account.id).then((s) => {
      setCurrent(s);
      setCape(s.capes.find((c) => c.active)?.id ?? null);
    });
  }, [account?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Який скін бібліотеки зараз на акаунті (за адресою текстури Mojang).
  const appliedId = useMemo(() => {
    if (!account || !lib || !current?.url) return null;
    const a = lib.applied[account.id];
    return a && a.url === current.url ? a.skin : null;
  }, [account, lib, current]);

  const currentPick: Pick | null = current?.skin ? { key: `current:${account?.id}:${current.url ?? ""}`, name: t("skins.current"), data: current.skin, slim: current.slim, source: "account" } : null;

  // Початковий вибір — те, що зараз на акаунті.
  useEffect(() => {
    if (pick || !lib) return;
    const fromLib = appliedId ? lib.skins.find((s) => s.id === appliedId) : undefined;
    if (fromLib) choose({ key: fromLib.id, libId: fromLib.id, name: fromLib.name, data: fromLib.data, slim: fromLib.slim, source: fromLib.source });
    else if (currentPick) choose(currentPick);
  }, [lib, appliedId, current]); // eslint-disable-line react-hooks/exhaustive-deps

  const choose = (p: Pick) => {
    setPick(p);
    setSlim(p.slim);
  };

  const addFile = async () => {
    const files = await open({ multiple: true, filters: [{ name: "PNG", extensions: ["png"] }] });
    const list = Array.isArray(files) ? files : files ? [files] : [];
    let last: LibSkin | null = null;
    for (const f of list) {
      try {
        last = await api.skinAddFile(f);
      } catch (e) {
        fail(e);
      }
    }
    await reload();
    if (last) choose({ key: last.id, libId: last.id, name: last.name, data: last.data, slim: last.slim, source: last.source });
  };

  // Стандартні скіни: по одній картці на персонажа, модель перемикає варіант текстури.
  const defaultGroups = useMemo(() => {
    const m = new Map<string, { wide?: LibSkin; slim?: LibSkin }>();
    for (const d of lib?.defaults ?? []) {
      const g = m.get(d.name) ?? {};
      if (d.slim) g.slim = d;
      else g.wide = d;
      m.set(d.name, g);
    }
    return [...m.entries()].map(([name, g]) => ({ name, ...g }));
  }, [lib]);

  const defaultPick = (d: LibSkin): Pick => ({ key: d.id, defaultId: d.id, name: d.name, data: d.data, slim: d.slim, source: "default" });

  const changeModel = (v: boolean) => {
    setSlim(v);
    if (pick?.libId) api.skinUpdate(pick.libId, { slim: v }).then(reload).catch(fail);
    if (pick?.defaultId) {
      const g = defaultGroups.find((x) => x.name === pick.name);
      const variant = v ? g?.slim : g?.wide;
      if (variant) setPick(defaultPick(variant));
    }
  };

  const apply = async () => {
    if (!account || !pick) return;
    setBusy(true);
    try {
      let skinId = pick.libId ?? pick.defaultId;
      // Поточний скін, якого ще немає в бібліотеці, спершу зберігаємо.
      if (!skinId) skinId = (await api.skinSaveCurrent(account.id)).id;
      await api.skinApply(account.id, skinId, slim);
      const capeNow = current?.capes.find((c) => c.active)?.id ?? null;
      if (cape !== capeNow) await api.setCape(account.id, cape);
      const fresh = await loadSkin(account.id, true);
      setCurrent(fresh);
      await reload();
      refreshAccounts();
      toast("success", t("skins.applied_toast"));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const remove = async (s: LibSkin) => {
    if (!(await confirmDialog({ title: t("skins.delete", { name: s.name }), danger: true }))) return;
    await api.skinRemove(s.id).catch(fail);
    if (pick?.libId === s.id) setPick(null);
    reload();
  };

  if (accounts.length === 0) {
    return (
      <div className="page">
        <div className="page-head">
          <h1>{t("skins.title")}</h1>
        </div>
        <div className="card">
          <Empty icon={<UserRound size={40} className="faint" />} title={t("acc.empty")} text={t("acc.empty_text")}>
            <button className="btn primary" onClick={() => setState({ addAccountOpen: true })}>
              <Plus size={16} />
              {t("acc.add")}
            </button>
          </Empty>
        </div>
      </div>
    );
  }

  const accountSelect = accounts.length > 1 && (
    <div style={{ width: 240 }}>
      <Select
        value={account?.id ?? null}
        options={accounts.map((a) => ({ value: a.id, label: a.name, sub: a.kind === "microsoft" ? "Microsoft" : t("acc.offline"), icon: <SkinHead accountId={a.id} name={a.name} size={20} /> }))}
        onChange={(v) => {
          setAccountId(v);
          setPick(null);
        }}
      />
    </div>
  );

  // Скін зберігають сервери Mojang — офлайн-акаунту його ніде поставити, тож і бібліотека йому ні до чого.
  if (!ms) {
    return (
      <div className="page">
        <div className="page-head">
          <h1>{t("skins.title")}</h1>
          {accountSelect}
          <button className="btn" onClick={() => navigate({ page: "skin-editor" })}>
            <Brush size={15} />
            {t("editor.title")}
          </button>
        </div>
        <div className="card">
          <Empty icon={<SkinHead accountId={account?.id} name={account?.name} size={56} />} title={t("skins.offline_title")} text={t("skins.offline_note")}>
            <button className="btn primary" onClick={() => setState({ addAccountOpen: true })}>
              <Plus size={16} />
              {t("acc.login_ms")}
            </button>
          </Empty>
        </div>
      </div>
    );
  }

  const capeUrl = cape ? (current?.capes.find((c) => c.id === cape)?.url ?? null) : null;
  const pickedApplied = !!pick && ((pick.libId && pick.libId === appliedId) || pick.source === "account");
  const unchanged = pickedApplied && slim === (current?.slim ?? slim) && cape === (current?.capes.find((c) => c.active)?.id ?? null);

  return (
    <div className="page">
      <div className="page-head">
        <h1>{t("skins.title")}</h1>
        {accountSelect}
        <button className="btn" onClick={() => navigate({ page: "skin-editor", start: pick ? { name: pick.libId ? pick.name : t("editor.new_name"), data: pick.data, slim, libId: pick.libId } : undefined })}>
          <Brush size={15} />
          {t("editor.title")}
        </button>
        <button className="btn" onClick={() => setNickOpen(true)}>
          <Search size={15} />
          {t("skins.by_nick")}
        </button>
        <button className="btn primary" onClick={addFile}>
          <FileUp size={15} />
          {t("skins.add_file")}
        </button>
      </div>

      <div className="skins-layout">
        <aside className="card skin-editor">
          <Stage data={pick?.data ?? current?.skin ?? null} slim={slim} cape={capeUrl} />
          <div className="skin-editor-body">
            <div className="row" style={{ gap: 8 }}>
              <SkinHead accountId={account?.id} name={account?.name} size={24} />
              <b className="grow ellipsis">{pick?.name ?? account?.name}</b>
            </div>
            <div className="field">
              <label>{t("skins.model")}</label>
              <Seg
                value={slim ? "slim" : "classic"}
                onChange={(v) => changeModel(v === "slim")}
                options={[
                  { value: "classic", label: t("acc.classic") },
                  { value: "slim", label: t("acc.slim") },
                ]}
              />
            </div>
            {current && current.capes.length > 0 && (
              <div className="field">
                <label>{t("acc.capes")}</label>
                <div className="capes">
                  <button className={`cape ${cape === null ? "on" : ""}`} onClick={() => setCape(null)}>
                    {t("acc.no_cape")}
                  </button>
                  {current.capes.map((c) => (
                    <button key={c.id} className={`cape ${cape === c.id ? "on" : ""}`} onClick={() => setCape(c.id)} title={c.alias ?? ""}>
                      {c.url && <CapeFront url={c.url} />}
                      <small>{c.alias}</small>
                    </button>
                  ))}
                </div>
              </div>
            )}
            <button className="btn primary lg" disabled={!pick || busy || !!unchanged} onClick={apply}>
              {busy ? <div className="spinner" style={{ width: 16, height: 16 }} /> : <Check size={17} />}
              {unchanged ? t("skins.applied") : t("skins.apply")}
            </button>
          </div>
        </aside>

        <section className="skin-library">
          <h3>{t("skins.mine")}</h3>
          <div className="skin-grid">
            <button className="skin-card add" onClick={addFile}>
              <Plus size={26} />
              <span>{t("skins.add")}</span>
              <small className="faint">{t("skins.drop_hint")}</small>
            </button>
            {currentPick && !appliedId && (
              <SkinCard
                pick={currentPick}
                active={pick?.key === currentPick.key}
                applied
                onClick={() => choose(currentPick)}
                actions={
                  <button
                    className="btn ghost sm icon"
                    title={t("skins.save_current")}
                    onClick={() =>
                      api
                        .skinSaveCurrent(account!.id)
                        .then(reload)
                        .catch(fail)
                    }
                  >
                    <Save size={14} />
                  </button>
                }
              />
            )}
            {lib?.skins.map((s) => {
              const p: Pick = { key: s.id, libId: s.id, name: s.name, data: s.data, slim: s.slim, source: s.source };
              return (
                <SkinCard
                  key={s.id}
                  pick={p}
                  active={pick?.libId === s.id}
                  applied={s.id === appliedId}
                  onClick={() => choose(p)}
                  actions={
                    <>
                      <button className="btn ghost sm icon" title={t("editor.title")} onClick={() => navigate({ page: "skin-editor", start: { name: s.name, data: s.data, slim: s.slim, libId: s.id } })}>
                        <Brush size={13} />
                      </button>
                      <button className="btn ghost sm icon" title={t("skins.rename")} onClick={() => setRenaming(s.id)}>
                        <Pencil size={13} />
                      </button>
                      <button className="btn ghost sm icon" title={t("common.delete")} onClick={() => remove(s)}>
                        <Trash2 size={13} />
                      </button>
                    </>
                  }
                />
              );
            })}
          </div>

          {defaultGroups.length > 0 && (
            <>
              <h3 style={{ marginTop: 24 }}>{t("skins.defaults")}</h3>
              <div className="skin-grid">
                {defaultGroups.map((g) => {
                  const main = (g.name === "Alex" ? (g.slim ?? g.wide) : (g.wide ?? g.slim))!;
                  const p = defaultPick(main);
                  const on = !!pick?.defaultId && pick.name === g.name;
                  const applied = appliedId === g.wide?.id || appliedId === g.slim?.id;
                  return <SkinCard key={g.name} pick={p} active={on} applied={applied} onClick={() => choose(p)} />;
                })}
              </div>
            </>
          )}
        </section>
      </div>

      {nickOpen && (
        <NickModal
          onClose={() => setNickOpen(false)}
          onAdded={(s) => {
            reload();
            choose({ key: s.id, libId: s.id, name: s.name, data: s.data, slim: s.slim, source: s.source });
          }}
        />
      )}
      {renaming && lib && <RenameModal skin={lib.skins.find((s) => s.id === renaming)!} onClose={() => setRenaming(null)} onDone={reload} />}
    </div>
  );
}

function RenameModal({ skin, onClose, onDone }: { skin: LibSkin; onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState(skin.name);
  const save = async () => {
    await api.skinUpdate(skin.id, { name }).catch(fail);
    onDone();
    onClose();
  };
  return (
    <Modal
      title={t("skins.rename")}
      onClose={onClose}
      foot={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={!name.trim()} onClick={save}>
            {t("common.save")}
          </button>
        </>
      }
    >
      <input className="input" autoFocus value={name} maxLength={32} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save()} />
    </Modal>
  );
}

/** Передня частина плаща (10×16 пікселів текстури) для мініатюри. */
function CapeFront({ url }: { url: string }) {
  const [src, setSrc] = useState("");
  useEffect(() => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = 40;
      c.height = 64;
      const g = c.getContext("2d")!;
      g.imageSmoothingEnabled = false;
      const k = img.width / 64;
      g.drawImage(img, 1 * k, 1 * k, 10 * k, 16 * k, 0, 0, 40, 64);
      setSrc(c.toDataURL());
    };
    img.src = url;
  }, [url]);
  return src ? <img src={src} width={30} height={48} alt="" style={{ imageRendering: "pixelated" }} /> : null;
}
