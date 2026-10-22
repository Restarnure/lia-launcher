import { useEffect, useMemo, useState } from "react";
import { Heart } from "lucide-react";
import { api, LOADERS, type Loader, type LoaderVersion, type VersionEntry } from "../lib/api";
import { BLOCK_KEYS, blockImage } from "../lib/blocks";
import { t, type Key } from "../lib/i18n";
import { fail, navigate, refreshInstances, setState } from "../lib/store";
import { loaderName, Modal, Seg, Select } from "./ui";

let versionsCache: VersionEntry[] | null = null;
const gamesCache = new Map<Loader, Promise<string[]>>();

export async function loadVersions(): Promise<VersionEntry[]> {
  if (!versionsCache) versionsCache = (await api.versions()).versions;
  return versionsCache;
}

export function loaderGames(l: Loader): Promise<string[]> {
  if (!gamesCache.has(l)) gamesCache.set(l, api.loaderGames(l).catch(() => []));
  return gamesCache.get(l)!;
}

type Filter = "release" | "snapshot" | "old";

function matches(v: VersionEntry, f: Filter) {
  return f === "release" ? v.kind === "release" : f === "snapshot" ? v.kind === "snapshot" : v.kind === "old_beta" || v.kind === "old_alpha";
}

const DONATE: Partial<Record<Loader, string>> = {
  forge: "https://www.patreon.com/LexManos",
  neoforge: "https://opencollective.com/neoforged",
};

/** Вибір версії гри + модлоадера (спільне для створення збірки й налаштувань). */
export function VersionPicker(props: {
  mc: string | null;
  loader: Loader;
  loaderVersion: string | null;
  onChange: (v: { mc: string | null; loader: Loader; loaderVersion: string | null }) => void;
}) {
  const [versions, setVersions] = useState<VersionEntry[] | null>(versionsCache);
  const [filter, setFilter] = useState<Filter>("release");
  const [supported, setSupported] = useState<Set<string> | null>(null);
  const [lv, setLv] = useState<LoaderVersion[] | null>(null);
  const { mc, loader, loaderVersion, onChange } = props;

  useEffect(() => {
    loadVersions()
      .then((v) => {
        setVersions(v);
        if (!mc) onChange({ mc: v.find((x) => x.kind === "release")?.id ?? null, loader, loaderVersion: null });
      })
      .catch(fail);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    setSupported(null);
    if (loader === "vanilla") return;
    let live = true;
    loaderGames(loader).then((g) => live && setSupported(new Set(g)));
    return () => {
      live = false;
    };
  }, [loader]);

  useEffect(() => {
    setLv(null);
    if (loader === "vanilla" || !mc) return;
    let live = true;
    api
      .loaderVersions(loader, mc)
      .then((list) => {
        if (!live) return;
        setLv(list);
        const keep = list.some((x) => x.id === loaderVersion);
        if (!keep) onChange({ mc, loader, loaderVersion: (list.find((x) => x.stable) ?? list[0])?.id ?? null });
      })
      .catch(() => live && setLv([]));
    return () => {
      live = false;
    };
  }, [loader, mc]); // eslint-disable-line react-hooks/exhaustive-deps

  const options = useMemo(() => {
    if (!versions) return [];
    return versions
      .filter((v) => matches(v, filter) || v.id === mc)
      .filter((v) => !supported || supported.has(v.id) || v.id === mc)
      .map((v) => ({ value: v.id, label: v.id, sub: v.kind === "release" ? undefined : v.kind.replace("old_", ""), search: v.id }));
  }, [versions, filter, supported, mc]);

  const unsupported = loader !== "vanilla" && supported && mc && !supported.has(mc);

  return (
    <>
      <div className="field">
        <label>{t("new.loader")}</label>
        <div className="loader-grid">
          {LOADERS.map((l) => (
            <button key={l} className={`loader-card ${l} ${l === loader ? "on" : ""}`} onClick={() => onChange({ mc, loader: l, loaderVersion: null })}>
              <b>{loaderName(l)}</b>
              <small>{t(`new.loader_desc.${l}` as Key)}</small>
            </button>
          ))}
        </div>
      </div>
      <div className="version-row">
        <div className="field" style={{ flex: "2 1 300px" }}>
          <label>{t("new.version")}</label>
          <div className="row">
            <Select className="grow" value={mc} options={options} onChange={(v) => onChange({ mc: v, loader, loaderVersion: null })} searchable placeholder={t("common.loading")} />
            <Seg<Filter>
              value={filter}
              onChange={setFilter}
              options={[
                { value: "release", label: t("new.releases") },
                { value: "snapshot", label: t("new.snapshots") },
                { value: "old", label: t("new.old") },
              ]}
            />
          </div>
        </div>
        {loader !== "vanilla" && (
          <div className="field" style={{ flex: "1 1 200px" }}>
            <label>{t("new.loader_version")}</label>
            <Select
              value={loaderVersion}
              disabled={!lv || lv.length === 0}
              placeholder={lv ? "—" : t("common.loading")}
              searchable={(lv?.length ?? 0) > 10}
              options={(lv ?? []).map((x, i) => ({
                value: x.id,
                label: x.name,
                sub: i === 0 && x.stable ? t("new.latest") : x.stable && (lv ?? []).findIndex((y) => y.stable) === i ? t("new.recommended") : undefined,
                search: x.name,
              }))}
              onChange={(v) => onChange({ mc, loader, loaderVersion: v })}
            />
          </div>
        )}
      </div>
      {unsupported && <p className="error-text">{t("new.unsupported", { loader: loaderName(loader), v: mc! })}</p>}
      {lv && lv.length === 0 && !unsupported && loader !== "vanilla" && <p className="error-text">{t("new.unsupported", { loader: loaderName(loader), v: mc ?? "" })}</p>}
      {DONATE[loader] && (
        <button className="donate" onClick={() => api.openUrl(DONATE[loader]!).catch(fail)}>
          <Heart size={14} />
          {t("new.forge_support")}
        </button>
      )}
    </>
  );
}

export function IconPicker({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="icon-picker">
      {BLOCK_KEYS.map((k) => (
        <button key={k} className={value === `block:${k}` ? "on" : ""} onClick={() => onChange(`block:${k}`)} title={k}>
          <img src={blockImage(k, 64)} width={34} height={34} alt={k} />
        </button>
      ))}
    </div>
  );
}

export function NewInstanceModal() {
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const [icon, setIcon] = useState("block:grass");
  const [sel, setSel] = useState<{ mc: string | null; loader: Loader; loaderVersion: string | null }>({ mc: null, loader: "vanilla", loaderVersion: null });
  const [busy, setBusy] = useState(false);
  const close = () => setState({ newInstanceOpen: false });

  const auto = sel.mc ? `${sel.loader === "vanilla" ? "Minecraft" : loaderName(sel.loader)} ${sel.mc}` : "";
  const finalName = touched ? name : auto;

  const create = async () => {
    if (!sel.mc) return;
    setBusy(true);
    try {
      const inst = await api.instanceCreate({ name: finalName.trim() || auto, mcVersion: sel.mc, loader: sel.loader, loaderVersion: sel.loaderVersion, icon });
      await refreshInstances();
      navigate({ page: "instance", id: inst.id });
      close();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("new.title")}
      onClose={close}
      size="wide"
      foot={
        <>
          <button className="btn ghost" onClick={close}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={!sel.mc || busy || (sel.loader !== "vanilla" && !sel.loaderVersion)} onClick={create}>
            {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {t("common.create")}
          </button>
        </>
      }
    >
      <div className="row" style={{ gap: 16, alignItems: "flex-start" }}>
        <div className="new-icon">
          <img src={blockImage(icon.slice(6), 128)} width={84} height={84} alt="" />
        </div>
        <div className="grow" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div className="field">
            <label>{t("new.name")}</label>
            <input
              className="input"
              value={finalName}
              placeholder={t("new.name_ph")}
              maxLength={48}
              autoFocus
              onChange={(e) => {
                setTouched(true);
                setName(e.target.value);
              }}
            />
          </div>
          <IconPicker value={icon} onChange={setIcon} />
        </div>
      </div>
      <VersionPicker mc={sel.mc} loader={sel.loader} loaderVersion={sel.loaderVersion} onChange={setSel} />
    </Modal>
  );
}
