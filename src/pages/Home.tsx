import { useEffect, useMemo, useState } from "react";
import { ArrowRight, Copy, ExternalLink, Globe, Play, Plus, Server, Users } from "lucide-react";
import { api, type Featured, type NewsEntry, type ServerStatus } from "../lib/api";
import { getLang, t } from "../lib/i18n";
import { McText } from "../lib/mctext";
import { fail, navigate, saveSettings, setState, toast, useInstanceTask, useStore } from "../lib/store";
import { InstanceCard } from "../components/InstanceCard";
import { Empty, InstanceIcon, PlayButton, Select, stageLabel } from "../components/ui";

// Нотатки до оновлень потрібні лише для шапки: остання версія і її картинка.
let newsCache: NewsEntry[] | null = null;

export function Home() {
  const instances = useStore((s) => s.instances);
  const settings = useStore((s) => s.settings);
  const accounts = useStore((s) => s.accounts);
  const me = accounts.accounts.find((a) => a.id === accounts.selected);
  const [news, setNews] = useState<NewsEntry[] | null>(newsCache);

  useEffect(() => {
    if (newsCache) return;
    api
      .news()
      .then((n) => {
        newsCache = n;
        setNews(n);
      })
      .catch(() => {});
  }, []);

  const current = instances.find((i) => i.id === settings?.lastInstance) ?? instances[0];
  const latestRelease = news?.find((n) => n.kind === "release");
  const heroImage = latestRelease?.image ?? news?.[0]?.image;

  return (
    <div className="page home">
      <section className="hero">
        {heroImage && <div className="hero-bg" style={{ backgroundImage: `url("${heroImage}")` }} />}
        <div className="hero-shade" />
        <div className="hero-content">
          {latestRelease && (
            <span className="hero-chip">
              <span className="dot" />
              {t("home.latest", { v: latestRelease.version })}
            </span>
          )}
          <h1>{me ? t("home.greeting", { name: me.name }) : t("home.greeting_anon")}</h1>
          <p className="muted">{t("home.subtitle")}</p>
          {current ? (
            <div className="hero-play">
              <div className="hero-pick">
                <InstanceIcon inst={current} size={44} />
                <Select
                  className="grow"
                  value={current.id}
                  options={instances.map((i) => ({
                    value: i.id,
                    label: i.name,
                    sub: `${i.mcVersion}${i.loader !== "vanilla" ? ` · ${i.loader}` : ""}`,
                    search: i.name,
                  }))}
                  onChange={(id) => saveSettings({ lastInstance: id })}
                  searchable={instances.length > 8}
                  minWidth={300}
                />
              </div>
              <PlayButton inst={current} />
            </div>
          ) : (
            <div className="hero-play">
              <button className="play" onClick={() => setState({ newInstanceOpen: true })}>
                <Plus size={18} />
                <span>{t("home.create_first").toUpperCase()}</span>
              </button>
            </div>
          )}
        </div>
      </section>

      {settings?.showFeatured && <FeaturedServer />}

      {instances.length > 0 ? (
        <>
          <div className="section-title">
            <h2>{t("home.recent")}</h2>
            <button className="btn ghost sm" onClick={() => navigate({ page: "instances" })}>
              {t("home.all")}
              <ArrowRight size={15} />
            </button>
          </div>
          <div className="igrid">
            {instances.slice(0, 4).map((i) => (
              <InstanceCard key={i.id} inst={i} />
            ))}
          </div>
        </>
      ) : (
        <div className="card" style={{ marginTop: 22 }}>
          <Empty title={t("home.no_instances")} text={t("home.no_instances_text")}>
            <button className="btn primary" onClick={() => setState({ newInstanceOpen: true })}>
              <Plus size={16} />
              {t("home.create_first")}
            </button>
          </Empty>
        </div>
      )}

    </div>
  );
}

// ------------------------------------------------------------ закріплений сервер

function FeaturedServer() {
  const [f, setF] = useState<Featured | null>(null);
  const [status, setStatus] = useState<ServerStatus | null | "offline">(null);
  const [busy, setBusy] = useState(false);
  const instances = useStore((s) => s.instances);
  const inst = useMemo(() => instances.find((i) => i.featured === f?.id), [instances, f]);
  const task = useInstanceTask(inst?.id);
  const running = useStore((s) => (inst ? !!s.running[inst.id] : false));

  useEffect(() => {
    let live = true;
    let timer: number | undefined;
    api.featured().then((x) => {
      if (!live) return;
      setF(x);
      const ping = () =>
        api
          .ping(x.address, x.port)
          .then((s) => live && setStatus(s))
          .catch(() => live && setStatus("offline"));
      ping();
      timer = window.setInterval(ping, 30000);
    });
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  if (!f) return <div className="skeleton" style={{ height: 132, marginTop: 22, borderRadius: 18 }} />;

  const play = async () => {
    setBusy(true);
    try {
      await api.featuredPlay();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const online = status && status !== "offline" ? status : null;
  const desc = getLang() === "uk" ? f.descriptionUk : f.descriptionEn;
  const pct = task && task.total > 0 ? Math.floor((task.done / task.total) * 100) : 0;

  return (
    <section className="featured">
      <div className="featured-glow" />
      <div className="featured-icon">
        {online?.favicon ? <img src={online.favicon} alt="" /> : <InstanceIcon inst={{ id: "featured", icon: "featured:shotam" }} size={64} />}
      </div>
      <div className="featured-main">
        <div className="row" style={{ gap: 10 }}>
          <span className="badge yellow">
            <Server size={12} />
            {t("featured.badge")}
          </span>
          <h2>{f.name}</h2>
        </div>
        <div className="motd">{online ? <McText value={online.motd} /> : <span className="muted">{desc}</span>}</div>
        <div className="featured-stats">
          {status === null ? (
            <span className="faint">{t("featured.checking")}</span>
          ) : online ? (
            <>
              <span className="row" style={{ gap: 7 }}>
                <span className="dot" />
                <Users size={14} />
                {t("featured.online", { n: online.online, max: online.max })}
              </span>
              {online.latencyMs > 0 && <span>{t("featured.ping", { ms: online.latencyMs })}</span>}
            </>
          ) : (
            <span className="row" style={{ gap: 7 }}>
              <span className="dot off" />
              {t("featured.offline")}
            </span>
          )}
          <span>{t("featured.version", { v: f.mcVersion })}</span>
          <button
            className="ip"
            title={t("featured.copy_ip")}
            onClick={() => navigator.clipboard.writeText(f.port === 25565 ? f.address : `${f.address}:${f.port}`).then(() => toast("info", t("common.copied")))}
          >
            {f.address}
            <Copy size={12} />
          </button>
        </div>
      </div>
      <div className="featured-actions">
        {task ? (
          <button className="play busy" onClick={() => api.taskCancel(task.id)}>
            <div className={`fill ${task.total > 0 ? "" : "indeterminate"}`} style={{ width: `${pct}%` }} />
            <span>{task.total > 0 ? `${stageLabel(task.stage)} · ${pct}%` : stageLabel(task.stage)}</span>
          </button>
        ) : running && inst ? (
          <PlayButton inst={inst} />
        ) : (
          <button className="play blue-play" onClick={play} disabled={busy}>
            {busy ? <div className="spinner" /> : <Play size={18} fill="currentColor" />}
            <span>{t("featured.play", { name: f.name }).toUpperCase()}</span>
          </button>
        )}
        {f.website && (
          <button className="btn ghost sm" onClick={() => api.openUrl(f.website!).catch(fail)}>
            <Globe size={15} />
            {t("featured.site")}
            <ExternalLink size={13} />
          </button>
        )}
      </div>
    </section>
  );
}
