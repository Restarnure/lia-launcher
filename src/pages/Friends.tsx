import { useEffect, useState } from "react";
import { Check, Download, Gamepad2, RefreshCw, Rss, Search, UserMinus, UserPlus, Users, X } from "lucide-react";
import { api, type Instance, type LiaEvent, type LiaPack, type LiaUser } from "../lib/api";
import { ago } from "../lib/format";
import { t } from "../lib/i18n";
import { installPack, loadFriends, useLia } from "../lib/lia";
import { fail, getState, refreshInstances, setState, toast, useStore } from "../lib/store";
import { confirmDialog, Empty, LoaderBadge } from "../components/ui";
import { LiaIdCard } from "../components/LiaId";

/** Голова зі скіна ЛІЯ ID (8×8 обличчя + шар). */
export function LiaHead({ user, size = 36 }: { user: Pick<LiaUser, "skin" | "name">; size?: number }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!user.skin) return setSrc(null);
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = c.height = 8;
      const g = c.getContext("2d")!;
      g.drawImage(img, 8, 8, 8, 8, 0, 0, 8, 8);
      g.drawImage(img, 40, 8, 8, 8, 0, 0, 8, 8);
      setSrc(c.toDataURL());
    };
    img.src = user.skin;
  }, [user.skin]);
  return src ? (
    <img className="lia-head" src={src} width={size} height={size} alt="" />
  ) : (
    <span className="lia-head letter" style={{ width: size, height: size, fontSize: size * 0.45 }}>
      {user.name[0]?.toUpperCase()}
    </span>
  );
}

function status(u: LiaUser): { text: string; cls: string } {
  const p = u.presence;
  if (!p) return { text: t("lia.offline"), cls: "off" };
  if (p.state === "playing") {
    const g = p.game;
    const what = [g?.name, g?.mcVersion].filter(Boolean).join(" · ");
    return { text: p.world ? t("lia.hosting", { what }) : t("lia.playing", { what }), cls: p.world ? "hosting" : "playing" };
  }
  return { text: t("lia.online"), cls: "on" };
}

/** Знайти або підготувати збірку, щоб зайти у світ друга: та сама збірка ЛІЇ, або та сама версія й модлоадер. */
async function joinFriend(u: LiaUser) {
  const p = u.presence;
  if (!p?.world) return;
  const s = getState();
  const me = s.accounts.accounts.find((a) => a.id === s.accounts.selected);
  if (!me) {
    setState({ addAccountOpen: true });
    return;
  }
  // Хост грає з ліцензією звичайним запуском: його світ перевіряє гостей через Mojang.
  if (p.world.licensedOnly && me.kind === "offline") {
    toast("error", t("lia.join_licensed_only"), t("lia.join_licensed_only_text"));
    return;
  }
  const g = p.game ?? {};
  let inst: Instance | undefined;
  if (g.pack) {
    inst = s.instances.find((i) => i.modpack?.projectId === `lia:${g.pack}`);
    if (!inst) {
      const ok = await confirmDialog({ title: t("lia.join_install", { name: g.name ?? "" }), text: t("lia.join_install_text") });
      if (!ok) return;
      inst = await installPack(g.pack);
    }
  } else {
    inst = s.instances.find((i) => i.mcVersion === g.mcVersion && i.loader === (g.loader ?? "vanilla"));
    if (!inst && (g.loader ?? "vanilla") === "vanilla" && g.mcVersion) {
      inst = await api.instanceCreate({ name: `Minecraft ${g.mcVersion}`, mcVersion: g.mcVersion, loader: "vanilla" });
      await refreshInstances();
    }
  }
  if (!inst) {
    toast("error", t("lia.join_no_instance"), t("lia.join_no_instance_text", { what: `${g.mcVersion ?? ""} ${g.loader ?? ""}`.trim() }));
    return;
  }
  const [host, port] = p.world.address.split(":");
  await api.launch(inst.id, { host, port: Number(port) || 25565 });
}

function FriendRow({ u }: { u: LiaUser }) {
  const s = status(u);
  const [busy, setBusy] = useState(false);
  return (
    <div className="lia-friend">
      <div className="lia-avatar">
        <LiaHead user={u} size={38} />
        <span className={`lia-dot ${s.cls}`} />
      </div>
      <div className="grow" style={{ minWidth: 0 }}>
        <b className="ellipsis">{u.name}</b>
        <div className="faint ellipsis" style={{ fontSize: 12.5 }}>
          {s.text}
        </div>
      </div>
      {u.presence?.world && (
        <button
          className="btn primary sm"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            await joinFriend(u).catch(fail);
            setBusy(false);
          }}
        >
          <Gamepad2 size={14} />
          {t("lia.join")}
        </button>
      )}
      <button
        className="btn ghost sm icon"
        title={t("lia.remove")}
        onClick={async () => {
          if (!(await confirmDialog({ title: t("lia.remove_q", { name: u.name }), danger: true }))) return;
          await api.lia("DELETE", `/api/friends/${u.name}`).catch(fail);
          loadFriends();
        }}
      >
        <UserMinus size={15} />
      </button>
    </div>
  );
}

function AddFriend() {
  const [q, setQ] = useState("");
  const [found, setFound] = useState<LiaUser[]>([]);
  useEffect(() => {
    if (!/^[A-Za-z0-9_]{1,16}$/.test(q)) return setFound([]);
    const h = setTimeout(() => api.lia<LiaUser[]>("GET", `/api/users?q=${q}`).then(setFound).catch(() => setFound([])), 250);
    return () => clearTimeout(h);
  }, [q]);
  const friends = useLia((s) => s.friends);
  const known = new Set([...(friends?.friends ?? []), ...(friends?.outgoing ?? [])].map((f) => f.name));
  return (
    <div className="card pad lia-add">
      <div className="lia-search">
        <Search size={15} className="faint" />
        <input className="input" placeholder={t("lia.search")} value={q} maxLength={16} onChange={(e) => setQ(e.target.value.replace(/[^A-Za-z0-9_]/g, ""))} />
      </div>
      {found.map((u) => (
        <div key={u.name} className="lia-friend">
          <LiaHead user={u} size={30} />
          <b className="grow ellipsis">{u.name}</b>
          {known.has(u.name) ? (
            <span className="faint" style={{ fontSize: 12.5 }}>
              {t("lia.already")}
            </span>
          ) : (
            <button
              className="btn sm"
              onClick={async () => {
                await api.lia("POST", `/api/friends/${u.name}`).catch(fail);
                toast("success", t("lia.request_sent", { name: u.name }));
                loadFriends();
              }}
            >
              <UserPlus size={14} />
              {t("lia.add")}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

function PackCard({ p, onChange }: { p: LiaPack; onChange: () => void }) {
  const instances = useStore((s) => s.instances);
  const local = instances.find((i) => i.modpack?.projectId === `lia:${p.id}`);
  const outdated = local && !p.mine && local.modpack?.versionId !== String(p.version);
  const [busy, setBusy] = useState(false);
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      onChange();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="card lia-pack">
      <div className="row" style={{ gap: 8 }}>
        <b className="grow ellipsis">{p.name}</b>
        <span className="badge">v{p.version}</span>
      </div>
      <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
        <span className="badge">Minecraft {p.mcVersion}</span>
        <LoaderBadge loader={p.loader} />
      </div>
      {p.description && <p className="hint">{p.description}</p>}
      <div className="faint" style={{ fontSize: 12 }}>
        {p.mine ? t("lia.pack_mine") : p.owner} · {ago(p.updated)} · {t("lia.subscribers", { n: p.subscribers })}
      </div>
      {!p.mine && (
        <div className="row" style={{ gap: 6 }}>
          {!local ? (
            <button className="btn primary sm" disabled={busy} onClick={() => run(() => installPack(p.id))}>
              <Download size={14} />
              {t("lia.install")}
            </button>
          ) : outdated ? (
            <button className="btn primary sm" disabled={busy} onClick={() => run(() => api.liaUpdatePack(local.id).then(refreshInstances))}>
              <RefreshCw size={14} />
              {t("lia.update")}
            </button>
          ) : (
            <span className="badge green">
              <Check size={12} />
              {t("lia.installed")}
            </span>
          )}
          <button
            className={`btn sm ${p.subscribed ? "" : "ghost"}`}
            disabled={busy}
            onClick={() => run(() => api.lia(p.subscribed ? "DELETE" : "POST", `/api/packs/${p.id}/subscribe`))}
          >
            <Rss size={14} />
            {p.subscribed ? t("lia.subscribed") : t("lia.subscribe")}
          </button>
        </div>
      )}
    </div>
  );
}

function FeedItem({ e }: { e: LiaEvent }) {
  const text =
    e.kind === "friend"
      ? t("lia.ev_friend", { a: e.data.a ?? "", b: e.data.b ?? "" })
      : e.kind === "pack_published"
        ? t("lia.ev_published", { user: e.user, name: e.data.name ?? "" })
        : t("lia.ev_updated", { user: e.user, name: e.data.name ?? "", v: e.data.version ?? "" });
  return (
    <div className="lia-event">
      <span>{text}</span>
      <span className="faint">{ago(e.created)}</span>
    </div>
  );
}

export function Friends() {
  const session = useLia((s) => (s.me?.emailVerified ? s.session : null));
  const online = useLia((s) => s.online);
  const friends = useLia((s) => s.friends);
  const rev = useLia((s) => s.rev);
  const [packs, setPacks] = useState<LiaPack[] | null>(null);
  const [feed, setFeed] = useState<LiaEvent[]>([]);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!session) return;
    api.lia<LiaPack[]>("GET", "/api/packs").then(setPacks).catch(fail);
    api.lia<LiaEvent[]>("GET", "/api/feed").then(setFeed).catch(() => {});
  }, [session, rev, tick]);

  if (!session) {
    return (
      <div className="page">
        <div className="page-head">
          <h1>{t("nav.friends")}</h1>
        </div>
        <div className="card">
          <Empty icon={<Users size={40} className="faint" />} title={t("lia.need_title")} text={t("lia.need_text")} />
        </div>
        <div style={{ marginTop: 14 }}>
          <LiaIdCard />
        </div>
      </div>
    );
  }

  const sorted = [...(friends?.friends ?? [])].sort((a, b) => Number(!!b.presence) - Number(!!a.presence) || a.name.localeCompare(b.name));
  const reload = () => setTick((x) => x + 1);

  return (
    <div className="page">
      <div className="page-head">
        <h1>{t("nav.friends")}</h1>
        <span className={`badge ${online ? "green" : ""}`}>
          <span className="dot" style={{ width: 6, height: 6 }} />
          {session.name} · {online ? t("lia.connected") : t("lia.connecting")}
        </span>
      </div>

      <div className="lia-layout">
        <section className="lia-col">
          <AddFriend />
          {friends && friends.incoming.length > 0 && (
            <div className="card pad">
              <h3>{t("lia.requests")}</h3>
              {friends.incoming.map((u) => (
                <div key={u.name} className="lia-friend">
                  <LiaHead user={u} size={32} />
                  <b className="grow ellipsis">{u.name}</b>
                  <button className="btn primary sm" onClick={() => api.lia("POST", `/api/friends/${u.name}`).then(loadFriends).catch(fail)}>
                    <Check size={14} />
                    {t("lia.accept")}
                  </button>
                  <button className="btn ghost sm icon" title={t("lia.decline")} onClick={() => api.lia("DELETE", `/api/friends/${u.name}`).then(loadFriends).catch(fail)}>
                    <X size={15} />
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="card pad">
            <h3>{t("lia.friends", { n: sorted.length })}</h3>
            {sorted.length === 0 ? <p className="hint">{t("lia.no_friends")}</p> : sorted.map((u) => <FriendRow key={u.name} u={u} />)}
            {friends && friends.outgoing.length > 0 && (
              <p className="hint" style={{ marginTop: 10 }}>
                {t("lia.outgoing", { names: friends.outgoing.map((u) => u.name).join(", ") })}
              </p>
            )}
          </div>
        </section>

        <section className="lia-col wide">
          <h3>{t("lia.packs")}</h3>
          {packs && packs.length === 0 && <p className="hint">{t("lia.no_packs")}</p>}
          <div className="lia-packs">{packs?.map((p) => <PackCard key={p.id} p={p} onChange={reload} />)}</div>
          <h3 style={{ marginTop: 22 }}>{t("lia.feed")}</h3>
          <div className="card pad">{feed.length === 0 ? <p className="hint">{t("lia.no_feed")}</p> : feed.map((e) => <FeedItem key={e.id} e={e} />)}</div>
        </section>
      </div>
    </div>
  );
}
