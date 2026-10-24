import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderSearch, Globe2 } from "lucide-react";
import { api, type Loader, type McScan } from "../lib/api";
import { bytes } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, navigate, refreshInstances, toast } from "../lib/store";
import { VersionPicker } from "./NewInstance";
import { Check, Modal } from "./ui";

export function MigrateModal({ onClose }: { onClose: () => void }) {
  const [scan, setScan] = useState<McScan | null>(null);
  const [saves, setSaves] = useState<Set<string>>(new Set());
  const [opts, setOpts] = useState({ resourcepacks: true, shaderpacks: true, mods: false, options: true });
  const [sel, setSel] = useState<{ mc: string | null; loader: Loader; loaderVersion: string | null }>({ mc: null, loader: "vanilla", loaderVersion: null });
  const [name, setName] = useState(t("mig.name"));
  const [busy, setBusy] = useState(false);
  const [pickerKey, setPickerKey] = useState(0);

  const load = (path?: string) => {
    setScan(null);
    api
      .mcdirScan(path)
      .then((s) => {
        setScan(s);
        setSaves(new Set(s.saves.map((x) => x.folder)));
        // Моди беремо завжди, коли вони є: прапорець лишається вимкненим лише для ванільної збірки.
        setOpts((o) => ({ ...o, mods: s.mods > 0 }));
        if (s.lastVersion) {
          const [mc, loader] = s.lastVersion;
          setSel({ mc, loader: loader as Loader, loaderVersion: null });

          setPickerKey((k) => k + 1);
        }
      })
      .catch(fail);
  };

  useEffect(() => load(), []);

  const run = async () => {
    if (!scan || !sel.mc) return;
    setBusy(true);
    try {
      const inst = await api.mcdirImport(
        scan.path,
        { name, mcVersion: sel.mc, loader: sel.loader, loaderVersion: sel.loaderVersion, icon: "block:grass" },
        { saves: [...saves], ...opts, mods: opts.mods && sel.loader !== "vanilla" },
      );
      await refreshInstances();
      toast("success", t("mig.done", { name: inst.name }));
      navigate({ page: "instance", id: inst.id, tab: "worlds" });
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const toggleSave = (f: string) =>
    setSaves((s) => {
      const n = new Set(s);
      if (n.has(f)) n.delete(f);
      else n.add(f);
      return n;
    });

  return (
    <Modal
      title={t("mig.title")}
      onClose={onClose}
      size="wide"
      foot={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={!scan?.exists || !sel.mc || busy || (sel.loader !== "vanilla" && !sel.loaderVersion)} onClick={run}>
            {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {t("mig.import")}
          </button>
        </>
      }
    >
      <p className="muted">{t("mig.text")}</p>
      <div className="field">
        <label>{t("mig.folder")}</label>
        <div className="row">
          <div className="path-box mono grow">{scan?.path ?? "…"}</div>
          <button
            className="btn"
            onClick={async () => {
              const p = await open({ directory: true, multiple: false });
              if (typeof p === "string") load(p);
            }}
          >
            <FolderSearch size={15} />
            {t("set.change")}
          </button>
        </div>
      </div>
      {scan === null ? (
        <div className="skeleton" style={{ height: 120 }} />
      ) : !scan.exists ? (
        <p className="error-text">{t("mig.not_found")}</p>
      ) : (
        <>
          {scan.saves.length > 0 && (
            <div className="field">
              <label>
                {t("mig.worlds")} ({saves.size}/{scan.saves.length})
              </label>
              <div className="mig-worlds">
                {scan.saves.map((s) => (
                  <Check key={s.folder} on={saves.has(s.folder)} onChange={() => toggleSave(s.folder)}>
                    <Globe2 size={14} className="faint" />
                    <span className="grow ellipsis">{s.folder}</span>
                    <small className="faint">{bytes(s.size)}</small>
                  </Check>
                ))}
              </div>
            </div>
          )}
          <div className="field">
            <label>{t("mig.extra")}</label>
            <div className="mig-opts">
              <Check on={opts.resourcepacks && scan.resourcepacks > 0} disabled={!scan.resourcepacks} onChange={(v) => setOpts({ ...opts, resourcepacks: v })}>
                {t("mig.resourcepacks", { n: scan.resourcepacks })}
              </Check>
              <Check on={opts.shaderpacks && scan.shaderpacks > 0} disabled={!scan.shaderpacks} onChange={(v) => setOpts({ ...opts, shaderpacks: v })}>
                {t("mig.shaderpacks", { n: scan.shaderpacks })}
              </Check>
              <Check on={opts.mods && sel.loader !== "vanilla" && scan.mods > 0} disabled={!scan.mods || sel.loader === "vanilla"} onChange={(v) => setOpts({ ...opts, mods: v })}>
                {t("mig.mods", { n: scan.mods })}
              </Check>
              <Check on={opts.options && (scan.hasOptions || scan.hasServers)} disabled={!scan.hasOptions && !scan.hasServers} onChange={(v) => setOpts({ ...opts, options: v })}>
                {t("mig.options")}
              </Check>
            </div>
          </div>
          {scan.lastVersion && <p className="hint">{t("mig.detected", { v: `${scan.lastVersion[0]}${scan.lastVersion[1] !== "vanilla" ? ` · ${scan.lastVersion[1]}` : ""}` })}</p>}
          <div className="field">
            <label>{t("new.name")}</label>
            <input className="input" value={name} maxLength={48} onChange={(e) => setName(e.target.value)} />
          </div>
          <VersionPicker key={pickerKey} mc={sel.mc} loader={sel.loader} loaderVersion={sel.loaderVersion} onChange={setSel} />
        </>
      )}
    </Modal>
  );
}
