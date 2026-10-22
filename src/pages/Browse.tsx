import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { Check, Download, ExternalLink, Heart, Package, Search, WifiOff } from "lucide-react";
import { api, type ContentKind, type Instance, type Json, type ProjectType, type SearchHit } from "../lib/api";
import { ago, compact, date } from "../lib/format";
import { getLang, t, type Key } from "../lib/i18n";
import { fail, navigate, toast, useStore } from "../lib/store";
import { Empty, InstanceIcon, LoaderBadge, Modal, Seg, Select } from "../components/ui";

const TYPES: ProjectType[] = ["mod", "modpack", "resourcepack", "shader"];

const CATEGORIES: Record<ProjectType, string[]> = {
  mod: ["optimization", "utility", "decoration", "adventure", "worldgen", "technology", "magic", "storage", "mobs", "equipment", "food", "library", "social", "transportation", "game-mechanics", "management"],
  modpack: ["adventure", "kitchen-sink", "lightweight", "optimization", "technology", "magic", "quests", "multiplayer", "combat", "challenging"],
  resourcepack: ["vanilla-like", "realistic", "simplistic", "themed", "cartoon", "16x", "32x", "64x", "128x", "gui", "fonts", "audio"],
  shader: ["vanilla-like", "realistic", "semi-realistic", "fantasy", "cartoon", "potato", "low", "medium", "high", "path-tracing", "pbr"],
};

const CAT_UK: Record<string, string> = {
  optimization: "Оптимізація",
  utility: "Утиліти",
  decoration: "Декор",
  adventure: "Пригоди",
  worldgen: "Генерація світу",
  technology: "Технології",
  magic: "Магія",
  storage: "Сховища",
  mobs: "Моби",
  equipment: "Спорядження",
  food: "Їжа",
  library: "Бібліотеки",
  social: "Спілкування",
  transportation: "Транспорт",
  "game-mechanics": "Механіки",
  management: "Керування",
  "kitchen-sink": "Усе й одразу",
  lightweight: "Легкі",
  quests: "Квести",
  multiplayer: "Мультиплеєр",
  combat: "Бої",
  challenging: "Складні",
  "vanilla-like": "Як ванільні",
  realistic: "Реалістичні",
  simplistic: "Прості",
  themed: "Тематичні",
  cartoon: "Мультяшні",
  gui: "Інтерфейс",
  fonts: "Шрифти",
  audio: "Звуки",
  "semi-realistic": "Напівреалістичні",
  fantasy: "Фентезі",
  potato: "Для слабких ПК",
  low: "Легкі",
  medium: "Середні",
  high: "Вимогливі",
  "path-tracing": "Трасування",
  pbr: "PBR",
};

const BRANDS: Record<string, string> = { fabric: "Fabric", forge: "Forge", neoforge: "NeoForge", quilt: "Quilt", babric: "Babric", iris: "Iris", optifine: "OptiFine", canvas: "Canvas", vanilla: "Vanilla", minecraft: "Minecraft" };

function catName(c: string) {
  if (BRANDS[c]) return BRANDS[c];
  return getLang() === "uk" ? (CAT_UK[c] ?? c) : c.replace(/-/g, " ").replace(/^./, (x) => x.toUpperCase());
}

const KIND: Record<string, ContentKind> = { mod: "mods", resourcepack: "resourcepacks", shader: "shaderpacks" };

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setV(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return v;
}

export function Browse({ type: initialType, instanceId }: { type?: ProjectType; instanceId?: string | null }) {
  const instances = useStore((s) => s.instances);
  const lastInstance = useStore((s) => s.settings?.lastInstance);
  const [type, setType] = useState<ProjectType>(initialType ?? "mod");
  // Без явного вибору — остання збірка: тоді «Встановити» працює одразу.
  const [target, setTarget] = useState<string | null>(instanceId ?? (instances.find((i) => i.id === lastInstance) ?? instances[0])?.id ?? null);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState("relevance");
  const [cats, setCats] = useState<string[]>([]);
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [offline, setOffline] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [open, setOpen] = useState<SearchHit | null>(null);
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const q = useDebounced(query, 300);
  const reqId = useRef(0);

  const inst = instances.find((i) => i.id === target) ?? null;
  const useTarget = type !== "modpack" && inst;

  useEffect(() => {
    if (initialType) setType(initialType);
    if (instanceId) setTarget(instanceId);
  }, [initialType, instanceId]);

  useEffect(() => setCats([]), [type]);

  const params = useMemo(
    () => ({
      query: q,
      projectType: type,
      gameVersion: useTarget ? inst!.mcVersion : null,
      loader: useTarget && type === "mod" ? inst!.loader : null,
      categories: cats,
      sort,
      limit: 20,
    }),
    [q, type, useTarget, inst, cats, sort],
  );

  useEffect(() => {
    const id = ++reqId.current;
    setHits(null);
    api
      .search({ ...params, offset: 0 })
      .then((r) => {
        if (id !== reqId.current) return;
        setHits(r.hits);
        setTotal(r.total_hits);
        setOffline(false);
      })
      .catch(() => {
        if (id !== reqId.current) return;
        setHits([]);
        setOffline(true);
      });
  }, [params]);

  const refreshInstalled = useCallback(() => {
    if (!useTarget) return setInstalled(new Set());
    api
      .content(inst!.id, KIND[type])
      .then((items) => setInstalled(new Set(items.map((i) => i.projectId).filter(Boolean) as string[])))
      .catch(() => setInstalled(new Set()));
  }, [useTarget, inst, type]);

  useEffect(() => refreshInstalled(), [refreshInstalled]);

  const more = async () => {
    if (!hits) return;
    setLoadingMore(true);
    try {
      const r = await api.search({ ...params, offset: hits.length });
      setHits([...hits, ...r.hits]);
    } catch (e) {
      fail(e);
    } finally {
      setLoadingMore(false);
    }
  };

  const install = async (hit: SearchHit, versionId?: string) => {
    setBusy((s) => new Set(s).add(hit.project_id));
    try {
      if (type === "modpack") {
        let vid = versionId;
        if (!vid) {
          const versions = await api.projectVersions(hit.project_id, [], []);
          vid = (versions.find((v: Json) => v.version_type === "release") ?? versions[0])?.id;
        }
        if (!vid) throw { code: "no_compatible_version", message: "" };
        const created = await api.installModpack(vid, hit.title);
        toast("success", t("browse.modpack_done", { name: created.name }));
        navigate({ page: "instance", id: created.id });
      } else if (inst) {
        const res = await api.install(inst.id, hit.project_id, versionId);
        toast("success", t("browse.installed_toast", { names: res.map((r) => r.title).join(", ") }));
        refreshInstalled();
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusy((s) => {
        const n = new Set(s);
        n.delete(hit.project_id);
        return n;
      });
    }
  };

  const instanceOptions = [
    { value: "", label: t("browse.no_instance") },
    ...instances.map((i) => ({ value: i.id, label: i.name, sub: `${i.mcVersion} · ${i.loader}`, search: i.name, icon: <InstanceIcon inst={i} size={20} /> })),
  ];

  return (
    <div className="page browse">
      <div className="page-head">
        <div className="grow">
          <h1>{t("browse.title")}</h1>
          <p className="muted" style={{ marginTop: 4 }}>
            {t("browse.subtitle")}
          </p>
        </div>
        {type !== "modpack" && (
          <div className="field" style={{ width: 280 }}>
            <label>{t("browse.for")}</label>
            <Select value={target ?? ""} options={instanceOptions} onChange={(v) => setTarget(v || null)} searchable={instances.length > 8} />
          </div>
        )}
      </div>

      <div className="browse-bar">
        <Seg<ProjectType> value={type} onChange={setType} options={TYPES.map((x) => ({ value: x, label: t(`browse.type.${x}` as Key) }))} />
        <div className="input-wrap grow">
          <Search size={16} />
          <input className="input" placeholder={t("browse.search_ph")} value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <Select
          value={sort}
          minWidth={210}
          className="sort"
          onChange={setSort}
          options={["relevance", "downloads", "follows", "newest", "updated"].map((s) => ({ value: s, label: t(`browse.sort.${s}` as Key) }))}
        />
      </div>

      <div className="chips">
        {CATEGORIES[type].map((c) => (
          <button key={c} className={`chip ${cats.includes(c) ? "on" : ""}`} onClick={() => setCats((cur) => (cur.includes(c) ? cur.filter((x) => x !== c) : [...cur, c]))}>
            {catName(c)}
          </button>
        ))}
      </div>

      {type !== "modpack" && !inst && <div className="hint-bar">{t("browse.no_instance_hint")}</div>}
      {type === "mod" && inst?.loader === "vanilla" && <div className="hint-bar warn">{t("browse.vanilla_mods")}</div>}

      {hits === null ? (
        <div className="results">
          {[0, 1, 2, 3, 4].map((i) => (
            <div key={i} className="skeleton" style={{ height: 96 }} />
          ))}
        </div>
      ) : offline ? (
        <Empty icon={<WifiOff size={36} className="faint" />} title={t("browse.offline")} />
      ) : hits.length === 0 ? (
        <Empty icon={<Search size={36} className="faint" />} title={t("browse.nothing")} text={t("browse.nothing_text")} />
      ) : (
        <>
          <p className="faint" style={{ fontSize: 12.5, margin: "4px 2px 10px" }}>
            {t("browse.results", { n: compact(total) })}
          </p>
          <div className="results">
            {hits.map((h) => {
              const isInstalled = installed.has(h.project_id);
              const isBusy = busy.has(h.project_id);
              const canInstall = type === "modpack" || (!!inst && !(type === "mod" && inst.loader === "vanilla"));
              return (
                <div key={h.project_id} className="result" onClick={() => setOpen(h)}>
                  <div className="result-icon">{h.icon_url ? <img src={h.icon_url} alt="" loading="lazy" /> : <Package size={26} className="faint" />}</div>
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="row" style={{ gap: 8 }}>
                      <b className="ellipsis">{h.title}</b>
                      <span className="faint ellipsis" style={{ fontSize: 12.5 }}>
                        {t("browse.by", { author: h.author })}
                      </span>
                    </div>
                    <p className="muted desc">{h.description}</p>
                    <div className="result-meta">
                      <span>
                        <Download size={13} />
                        {compact(h.downloads)}
                      </span>
                      <span>
                        <Heart size={13} />
                        {compact(h.follows)}
                      </span>
                      <span>{t("browse.updated", { when: ago(new Date(h.date_modified).getTime()) })}</span>
                      {(h.display_categories ?? h.categories).slice(0, 4).map((c) => (
                        <span key={c} className="tag">
                          {catName(c)}
                        </span>
                      ))}
                    </div>
                  </div>
                  <div onClick={(e) => e.stopPropagation()}>
                    {isInstalled ? (
                      <button className="btn sm" disabled>
                        <Check size={14} />
                        {t("common.installed")}
                      </button>
                    ) : (
                      <button className={`btn sm ${type === "modpack" ? "blue" : "primary"}`} disabled={!canInstall || isBusy} onClick={() => install(h)}>
                        {isBusy ? <div className="spinner" style={{ width: 13, height: 13 }} /> : <Download size={14} />}
                        {type === "modpack" ? t("browse.install_modpack") : t("common.install")}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          {hits.length < total && (
            <div className="row" style={{ justifyContent: "center", marginTop: 16 }}>
              <button className="btn" onClick={more} disabled={loadingMore}>
                {loadingMore && <div className="spinner" style={{ width: 14, height: 14 }} />}
                {t("browse.load_more")}
              </button>
            </div>
          )}
        </>
      )}
      {open && (
        <ProjectModal
          hit={open}
          type={type}
          inst={useTarget ? inst : null}
          installed={installed.has(open.project_id)}
          busy={busy.has(open.project_id)}
          onInstall={(vid) => install(open, vid)}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

function ProjectModal(props: { hit: SearchHit; type: ProjectType; inst: Instance | null; installed: boolean; busy: boolean; onInstall: (versionId?: string) => void; onClose: () => void }) {
  const { hit, inst, type } = props;
  const [project, setProject] = useState<Json | null>(null);
  const [versions, setVersions] = useState<Json[] | null>(null);
  const [tab, setTab] = useState<"desc" | "versions">("desc");
  const [shot, setShot] = useState(0);

  useEffect(() => {
    api.project(hit.project_id).then(setProject).catch(fail);
    const loaders = inst && type === "mod" ? (inst.loader === "quilt" ? ["quilt", "fabric"] : [inst.loader]) : [];
    const games = inst ? [inst.mcVersion] : [];
    api
      .projectVersions(hit.project_id, loaders, games)
      .then(setVersions)
      .catch(() => setVersions([]));
  }, [hit.project_id, inst, type]);

  const body = useMemo(() => {
    if (!project?.body) return "";
    const html = marked.parse(project.body as string, { async: false }) as string;
    return DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "iframe", "form", "script"], FORBID_ATTR: ["style"] });
  }, [project]);

  const gallery: string[] = (project?.gallery ?? []).map((g: Json) => g.url as string);
  const links: [string, string | undefined][] = [
    ["Modrinth", `https://modrinth.com/${project?.project_type ?? "project"}/${hit.slug}`],
    ["Source", project?.source_url],
    ["Wiki", project?.wiki_url],
    ["Discord", project?.discord_url],
  ];
  const canInstall = type === "modpack" || (!!inst && !(type === "mod" && inst.loader === "vanilla"));

  return (
    <Modal onClose={props.onClose} size="xwide">
      <div className="proj-head">
        <div className="result-icon lg">{hit.icon_url ? <img src={hit.icon_url} alt="" /> : <Package size={34} className="faint" />}</div>
        <div className="grow" style={{ minWidth: 0 }}>
          <h2>{hit.title}</h2>
          <p className="muted">{hit.description}</p>
          <div className="result-meta" style={{ marginTop: 8 }}>
            <span>{t("browse.by", { author: hit.author })}</span>
            <span>
              <Download size={13} />
              {compact(hit.downloads)} {t("browse.downloads")}
            </span>
            <span>
              <Heart size={13} />
              {compact(hit.follows)}
            </span>
          </div>
        </div>
        <div className="proj-actions">
          {props.installed ? (
            <button className="btn" disabled>
              <Check size={15} />
              {t("common.installed")}
            </button>
          ) : (
            <button className={`btn ${type === "modpack" ? "blue" : "primary"}`} disabled={!canInstall || props.busy} onClick={() => props.onInstall()}>
              {props.busy ? <div className="spinner" style={{ width: 14, height: 14 }} /> : <Download size={15} />}
              {type === "modpack" ? t("browse.install_modpack") : t("common.install")}
            </button>
          )}
          {inst && (
            <span className="faint row" style={{ gap: 6, fontSize: 12 }}>
              <InstanceIcon inst={inst} size={16} />
              {inst.name}
            </span>
          )}
        </div>
      </div>

      {gallery.length > 0 && (
        <div className="gallery">
          <img src={gallery[shot]} alt="" />
          {gallery.length > 1 && (
            <div className="gallery-thumbs">
              {gallery.slice(0, 10).map((g, i) => (
                <button key={g} className={i === shot ? "on" : ""} onClick={() => setShot(i)}>
                  <img src={g} alt="" loading="lazy" />
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <nav className="tabs" style={{ marginBottom: 0 }}>
        <button className={`tab ${tab === "desc" ? "on" : ""}`} onClick={() => setTab("desc")}>
          {t("browse.description")}
        </button>
        <button className={`tab ${tab === "versions" ? "on" : ""}`} onClick={() => setTab("versions")}>
          {t("browse.versions")}
          {versions && <span className="count">{versions.length}</span>}
        </button>
        <div className="grow" />
        {links
          .filter(([, u]) => u)
          .map(([name, u]) => (
            <button key={name} className="btn ghost sm" onClick={() => api.openUrl(u!).catch(fail)}>
              {name}
              <ExternalLink size={12} />
            </button>
          ))}
      </nav>

      {tab === "desc" ? (
        project ? (
          <div
            className="prose selectable"
            dangerouslySetInnerHTML={{ __html: body }}
            onClick={(e) => {
              const a = (e.target as HTMLElement).closest("a");
              if (a?.href) {
                e.preventDefault();
                api.openUrl(a.href).catch(fail);
              }
            }}
          />
        ) : (
          <div className="skeleton" style={{ height: 200 }} />
        )
      ) : versions === null ? (
        <div className="skeleton" style={{ height: 200 }} />
      ) : versions.length === 0 ? (
        <Empty title={t("browse.no_versions")} />
      ) : (
        <div className="list">
          {versions.slice(0, 60).map((v: Json) => (
            <div key={v.id} className="list-row">
              <span className={`badge ${v.version_type === "release" ? "green" : v.version_type === "beta" ? "yellow" : "red"}`}>{v.version_type}</span>
              <div className="grow" style={{ minWidth: 0 }}>
                <b className="ellipsis" style={{ display: "block" }}>
                  {v.name || v.version_number}
                </b>
                <small className="faint ellipsis" style={{ display: "block" }}>
                  {(v.game_versions as string[]).slice(-6).join(", ")} · {date(v.date_published)}
                </small>
              </div>
              <div className="row" style={{ gap: 4 }}>
                {(v.loaders as string[]).slice(0, 3).map((l) =>
                  ["fabric", "quilt", "forge", "neoforge"].includes(l) ? <LoaderBadge key={l} loader={l as never} /> : <span key={l} className="badge">{l}</span>,
                )}
              </div>
              <button className="btn sm" disabled={!canInstall || props.busy} onClick={() => props.onInstall(v.id)}>
                <Download size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}
