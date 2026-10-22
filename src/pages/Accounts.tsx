import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { IdleAnimation, SkinViewer, WalkingAnimation } from "skinview3d";
import { Check, Plus, Trash2, Upload, UserRound } from "lucide-react";
import { api, type Account, type SkinInfo } from "../lib/api";
import { t } from "../lib/i18n";
import { fail, refreshAccounts, setState, toast, useStore } from "../lib/store";
import { confirmDialog, Empty, loadSkin, SkinHead, Switch } from "../components/ui";

export function Accounts() {
  const { accounts, selected } = useStore((s) => s.accounts);
  const setAdding = (v: boolean) => setState({ addAccountOpen: v });
  const [viewId, setViewId] = useState<string | null>(null);
  const current = accounts.find((a) => a.id === (viewId ?? selected)) ?? accounts[0];

  const remove = async (a: Account) => {
    if (!(await confirmDialog({ title: t("acc.remove_title", { name: a.name }), text: t("acc.remove_text"), danger: true }))) return;
    await api.removeAccount(a.id).catch(fail);
    if (viewId === a.id) setViewId(null);
    refreshAccounts();
  };

  return (
    <div className="page">
      <div className="page-head">
        <h1>{t("acc.title")}</h1>
        <button className="btn primary" onClick={() => setAdding(true)}>
          <Plus size={16} />
          {t("acc.add")}
        </button>
      </div>
      {accounts.length === 0 ? (
        <div className="card">
          <Empty icon={<UserRound size={40} className="faint" />} title={t("acc.empty")} text={t("acc.empty_text")}>
            <button className="btn primary" onClick={() => setAdding(true)}>
              <Plus size={16} />
              {t("acc.add")}
            </button>
          </Empty>
        </div>
      ) : (
        <div className="acc-layout">
          <div className="acc-list">
            {accounts.map((a) => (
              <div
                key={a.id}
                className={`acc ${a.id === current?.id ? "viewing" : ""}`}
                onClick={() => {
                  setViewId(a.id);
                  if (a.id !== selected) api.selectAccount(a.id).then(refreshAccounts).catch(fail);
                }}
              >
                <SkinHead accountId={a.id} name={a.name} size={42} />
                <div className="grow" style={{ minWidth: 0 }}>
                  <b className="ellipsis" style={{ display: "block" }}>
                    {a.name}
                  </b>
                  <span className={`badge ${a.kind === "microsoft" ? "blue" : ""}`}>{a.kind === "microsoft" ? t("acc.microsoft") : t("acc.offline")}</span>
                </div>
                {a.id === selected && (
                  <span className="badge green">
                    <Check size={12} />
                    {t("acc.selected")}
                  </span>
                )}
                <button
                  className="btn ghost sm icon"
                  onClick={(e) => {
                    e.stopPropagation();
                    remove(a);
                  }}
                  title={t("common.delete")}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
          </div>
          {current && <SkinPanel key={current.id} account={current} />}
        </div>
      )}
    </div>
  );
}

function SkinPanel({ account }: { account: Account }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<SkinViewer | null>(null);
  const [info, setInfo] = useState<SkinInfo | null>(null);
  const [walk, setWalk] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    loadSkin(account.id).then((s) => live && setInfo(s));
    return () => {
      live = false;
    };
  }, [account.id]);

  useEffect(() => {
    if (!canvas.current) return;
    const v = new SkinViewer({ canvas: canvas.current, width: 300, height: 380 });
    v.zoom = 0.82;
    v.fov = 55;
    v.autoRotate = true;
    v.autoRotateSpeed = 0.6;
    v.controls.enableZoom = false;
    viewer.current = v;
    return () => {
      v.dispose();
      viewer.current = null;
    };
  }, []);

  useEffect(() => {
    const v = viewer.current;
    if (!v || !info) return;
    if (info.skin) v.loadSkin(info.skin, { model: info.slim ? "slim" : "default" });
    else v.loadSkin(null);
    const cape = info.capes.find((c) => c.active)?.url;
    if (cape) v.loadCape(cape);
    else v.loadCape(null);
  }, [info]);

  useEffect(() => {
    const v = viewer.current;
    if (!v) return;
    const anim = walk ? new WalkingAnimation() : new IdleAnimation();
    anim.speed = walk ? 0.55 : 1;
    v.animation = anim;
  }, [walk]);

  const changeSkin = async (slim: boolean) => {
    const path = await open({ multiple: false, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (typeof path !== "string") return;
    setBusy(true);
    try {
      await api.uploadSkin(account.id, path, slim);
      setInfo(await loadSkin(account.id, true));
      toast("success", t("acc.skin_done"));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const setCape = async (id: string | null) => {
    setBusy(true);
    try {
      await api.setCape(account.id, id);
      setInfo(await loadSkin(account.id, true));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const ms = account.kind === "microsoft";
  const activeCape = info?.capes.find((c) => c.active)?.id ?? null;

  return (
    <div className="skin-panel card">
      <div className="skin-stage">
        <canvas ref={canvas} />
        {!info && <div className="spinner" style={{ position: "absolute" }} />}
      </div>
      <div className="skin-info">
        <h2>{account.name}</h2>
        <p className="faint mono selectable" style={{ fontSize: 11.5 }}>
          {account.uuid}
        </p>
        <label className="row" style={{ gap: 10, fontSize: 13 }}>
          <Switch on={walk} onChange={setWalk} />
          {t("acc.walk")}
        </label>
        {ms ? (
          <>
            <div className="field">
              <label>{t("acc.change_skin")}</label>
              <div className="row">
                <button className="btn sm" disabled={busy} onClick={() => changeSkin(false)}>
                  <Upload size={14} />
                  {t("acc.classic")}
                </button>
                <button className="btn sm" disabled={busy} onClick={() => changeSkin(true)}>
                  <Upload size={14} />
                  {t("acc.slim")}
                </button>
              </div>
            </div>
            {info && info.capes.length > 0 && (
              <div className="field">
                <label>{t("acc.capes")}</label>
                <div className="capes">
                  <button className={`cape ${activeCape === null ? "on" : ""}`} disabled={busy} onClick={() => setCape(null)}>
                    {t("acc.no_cape")}
                  </button>
                  {info.capes.map((c) => (
                    <button key={c.id} className={`cape ${c.active ? "on" : ""}`} disabled={busy} onClick={() => setCape(c.id)} title={c.alias ?? ""}>
                      {c.url ? <CapeFront url={c.url} /> : null}
                      <small>{c.alias}</small>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </>
        ) : (
          <p className="hint">{info?.source === "mojang_name" ? t("acc.skin_by_name") : t("acc.skin_default")}</p>
        )}
      </div>
    </div>
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
