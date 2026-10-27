import { useState } from "react";
import { LogOut, Users } from "lucide-react";
import { api, type Instance } from "../lib/api";
import { t } from "../lib/i18n";
import { refreshLia, useLia } from "../lib/lia";
import { fail, launch, toast, useStore } from "../lib/store";
import { confirmDialog } from "./ui";

/** ЛІЯ ID — акаунт лаунчера: вхід/реєстрація або хто увійшов. Окремо від ігрових акаунтів. */
export function LiaIdCard() {
  const session = useLia((s) => s.session);
  const online = useLia((s) => s.online);
  const [name, setName] = useState("");
  const [pass, setPass] = useState("");
  const [isNew, setIsNew] = useState(false);
  const [busy, setBusy] = useState(false);
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(name) && pass.length >= (isNew ? 8 : 1);

  const go = async () => {
    setBusy(true);
    try {
      const s = await api.liaLogin(name, pass, isNew);
      setPass("");
      await refreshLia();
      toast("success", t("lia.signed_in", { name: s.name }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  if (session) {
    return (
      <div className="card pad lia-id">
        <div className="lia-logo">
          <Users size={18} />
        </div>
        <div className="grow" style={{ minWidth: 0 }}>
          <b>{t("acc.lia")}</b>
          <div className="faint" style={{ fontSize: 12.5 }}>
            {t("lia.signed_in_as", { name: session.name })} · {online ? t("lia.connected") : t("lia.connecting")}
          </div>
        </div>
        <button
          className="btn ghost sm"
          onClick={async () => {
            if (!(await confirmDialog({ title: t("lia.logout_q") }))) return;
            await api.liaLogout().catch(fail);
            refreshLia();
          }}
        >
          <LogOut size={14} />
          {t("lia.logout")}
        </button>
      </div>
    );
  }

  return (
    <div className="card pad lia-id column">
      <div className="row" style={{ alignItems: "flex-start", gap: 12 }}>
        <div className="lia-logo">
          <Users size={18} />
        </div>
        <div className="grow">
          <b>{t("acc.lia")}</b>
          <p className="hint">{t("acc.lia_desc")}</p>
        </div>
      </div>
      <div className="row">
        <input className="input" placeholder={t("acc.lia_nick")} value={name} maxLength={16} onChange={(e) => setName(e.target.value.replace(/[^A-Za-z0-9_]/g, ""))} />
        <input
          className="input"
          type="password"
          placeholder={isNew ? t("acc.lia_pass_new") : t("acc.lia_pass")}
          value={pass}
          maxLength={128}
          onChange={(e) => setPass(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && valid && go()}
        />
        <button className="btn primary" disabled={!valid || busy} onClick={go}>
          {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
          {isNew ? t("acc.lia_register") : t("acc.lia_login")}
        </button>
      </div>
      <a
        href="#"
        className="hint"
        onClick={(e) => {
          e.preventDefault();
          setIsNew((v) => !v);
        }}
      >
        {isNew ? t("acc.lia_have") : t("acc.lia_new")}
      </a>
    </div>
  );
}

/**
 * «Грати з друзями» — лише для ліцензії: офлайн-нік з ЛІЯ ID і так входить через ЛІЮ.
 * Ліцензія в цьому запуску входить через ЛІЮ, тож у її LAN-світ зайдуть і пірати.
 */
export function FriendsPlayButton({ inst }: { inst: Instance }) {
  const session = useLia((s) => s.session);
  const running = useStore((s) => !!s.running[inst.id]);
  const busy = useStore((s) => Object.values(s.tasks).some((x) => x.tag === inst.id && x.state === "running"));
  const ms = useStore((s) => s.accounts.accounts.find((a) => a.id === s.accounts.selected)?.kind === "microsoft");
  if (!session || !ms || running || busy) return null;
  return (
    <button className="btn friends-play" title={t("play.friends_hint")} onClick={() => launch(inst.id, undefined, undefined, true)}>
      <Users size={16} />
      {t("play.friends")}
    </button>
  );
}
