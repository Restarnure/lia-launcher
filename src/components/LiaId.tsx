import { useEffect, useState } from "react";
import { KeyRound, LogOut, Mail, MonitorSmartphone, ShieldCheck, Trash2, Users } from "lucide-react";
import { api, type Instance, type LiaDevice } from "../lib/api";
import { ago } from "../lib/format";
import { t } from "../lib/i18n";
import { refreshLia, useLia } from "../lib/lia";
import { fail, launch, toast, useStore } from "../lib/store";
import { confirmDialog, Modal } from "./ui";

const NICK = /^[A-Za-z0-9_]{3,16}$/;
const EMAIL = /^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/;

/** Кнопка з крутилкою, поки йде дія. */
function useBusy() {
  const [busy, setBusy] = useState(false);
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      return true;
    } catch (e) {
      fail(e);
      return false;
    } finally {
      setBusy(false);
    }
  };
  return [busy, run] as const;
}

const Spin = () => <div className="spinner" style={{ width: 14, height: 14 }} />;

function Head({ title, text }: { title: string; text?: string }) {
  return (
    <div className="row" style={{ alignItems: "flex-start", gap: 12 }}>
      <div className="lia-logo">
        <Users size={18} />
      </div>
      <div className="grow" style={{ minWidth: 0 }}>
        <b>{title}</b>
        {text && <p className="hint">{text}</p>}
      </div>
    </div>
  );
}

/** Вхід, реєстрація, відновлення пароля. */
function SignIn() {
  const [mode, setMode] = useState<"login" | "register" | "forgot">("login");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [pass, setPass] = useState("");
  const [pass2, setPass2] = useState("");
  const [code, setCode] = useState("");
  const [sent, setSent] = useState(false);
  const [busy, run] = useBusy();

  const passOk = pass.length >= 8 && pass === pass2;
  const valid =
    mode === "login" ? name.trim().length >= 3 && pass.length > 0 : mode === "register" ? NICK.test(name) && EMAIL.test(email) && passOk : sent ? /^\d{6}$/.test(code) && passOk : name.trim().length >= 3;

  const go = () =>
    run(async () => {
      if (mode === "login") {
        await api.liaLogin(name, pass);
      } else if (mode === "register") {
        await api.liaLogin(name, pass, email);
      } else if (!sent) {
        await api.liaPublic("/api/password/forgot", { login: name });
        setSent(true);
        toast("info", t("lia.code_sent"), t("lia.code_sent_text"));
        return;
      } else {
        await api.liaPublic("/api/password/reset", { login: name, code, password: pass });
        toast("success", t("lia.reset_done"));
        setMode("login");
        setSent(false);
        setCode("");
        setPass2("");
        return;
      }
      setPass("");
      setPass2("");
      await refreshLia();
    });

  const switchTo = (m: typeof mode) => {
    setMode(m);
    setSent(false);
    setCode("");
    setPass("");
    setPass2("");
  };

  return (
    <div className="card pad lia-id column">
      <Head title={t("acc.lia")} text={t("acc.lia_desc")} />
      <div className="lia-form" onKeyDown={(e) => e.key === "Enter" && valid && !busy && go()}>
        <input
          className="input"
          placeholder={mode === "register" ? t("acc.lia_nick") : t("lia.login_field")}
          value={name}
          maxLength={mode === "register" ? 16 : 254}
          onChange={(e) => setName(mode === "register" ? e.target.value.replace(/[^A-Za-z0-9_]/g, "") : e.target.value)}
          disabled={mode === "forgot" && sent}
        />
        {mode === "register" && <input className="input" type="email" placeholder={t("lia.email")} value={email} maxLength={254} onChange={(e) => setEmail(e.target.value)} />}
        {mode === "forgot" && sent && <input className="input mono" inputMode="numeric" placeholder={t("lia.code")} value={code} maxLength={6} onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} />}
        {(mode !== "forgot" || sent) && (
          <input
            className="input"
            type="password"
            placeholder={mode === "login" ? t("acc.lia_pass") : t("acc.lia_pass_new")}
            value={pass}
            maxLength={128}
            onChange={(e) => setPass(e.target.value)}
          />
        )}
        {(mode === "register" || (mode === "forgot" && sent)) && (
          <input className="input" type="password" placeholder={t("lia.pass_repeat")} value={pass2} maxLength={128} onChange={(e) => setPass2(e.target.value)} />
        )}
        {(mode === "register" || (mode === "forgot" && sent)) && pass2.length > 0 && pass !== pass2 && <p className="error-text">{t("lia.pass_mismatch")}</p>}
        {mode === "register" && <p className="hint">{t("lia.pass_rules")}</p>}
        <button className="btn primary" disabled={!valid || busy} onClick={go}>
          {busy && <Spin />}
          {mode === "login" ? t("acc.lia_login") : mode === "register" ? t("acc.lia_register") : sent ? t("lia.reset") : t("lia.send_code")}
        </button>
      </div>
      <div className="row" style={{ gap: 14, flexWrap: "wrap" }}>
        {mode !== "login" && <a href="#" className="hint" onClick={(e) => (e.preventDefault(), switchTo("login"))}>{t("acc.lia_have")}</a>}
        {mode !== "register" && <a href="#" className="hint" onClick={(e) => (e.preventDefault(), switchTo("register"))}>{t("acc.lia_new")}</a>}
        {mode === "login" && <a href="#" className="hint" onClick={(e) => (e.preventDefault(), switchTo("forgot"))}>{t("lia.forgot")}</a>}
      </div>
    </div>
  );
}

/** Пошта ще не підтверджена (або її немає — акаунти з 1.1.x): код з листа. */
function Verify() {
  const me = useLia((s) => s.me);
  const [code, setCode] = useState("");
  const [changing, setChanging] = useState(!me?.email);
  const [email, setEmail] = useState("");
  const [pass, setPass] = useState("");
  const [busy, run] = useBusy();
  if (!me) return null;

  return (
    <div className="card pad lia-id column">
      <Head title={t("lia.verify_title")} text={me.email && !changing ? t("lia.verify_text", { email: me.email }) : t("lia.add_email_text")} />
      {changing ? (
        <div className="lia-form">
          <input className="input" type="email" placeholder={t("lia.email")} value={email} maxLength={254} onChange={(e) => setEmail(e.target.value)} />
          <input className="input" type="password" placeholder={t("acc.lia_pass")} value={pass} maxLength={128} onChange={(e) => setPass(e.target.value)} />
          <button
            className="btn primary"
            disabled={!EMAIL.test(email) || !pass || busy}
            onClick={() =>
              run(async () => {
                await api.lia("POST", "/api/email/change", { email, password: pass });
                setChanging(false);
                setPass("");
                toast("info", t("lia.code_sent"), t("lia.code_sent_to", { email }));
                await refreshLia();
              })
            }
          >
            {busy && <Spin />}
            {t("lia.send_code")}
          </button>
        </div>
      ) : (
        <div className="lia-form">
          <input className="input mono" inputMode="numeric" placeholder={t("lia.code")} value={code} maxLength={6} onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} />
          <button
            className="btn primary"
            disabled={!/^\d{6}$/.test(code) || busy}
            onClick={() =>
              run(async () => {
                await api.lia("POST", "/api/email/verify", { code });
                toast("success", t("lia.verified"));
                await refreshLia();
              })
            }
          >
            {busy && <Spin />}
            {t("lia.confirm")}
          </button>
        </div>
      )}
      <div className="row" style={{ gap: 14, flexWrap: "wrap" }}>
        {!changing && (
          <a href="#" className="hint" onClick={(e) => (e.preventDefault(), run(() => api.lia("POST", "/api/email/resend").then(() => toast("info", t("lia.code_sent")))))}>
            {t("lia.resend")}
          </a>
        )}
        <a href="#" className="hint" onClick={(e) => (e.preventDefault(), setChanging((v) => !v))}>
          {changing && me.email ? t("lia.have_code") : t("lia.other_email")}
        </a>
        <a href="#" className="hint" onClick={(e) => (e.preventDefault(), api.liaLogout().then(() => refreshLia()).catch(fail))}>
          {t("lia.logout")}
        </a>
      </div>
    </div>
  );
}

/** ЛІЯ ID — акаунт лаунчера: вхід, підтвердження пошти або хто увійшов. Окремо від ігрових акаунтів. */
export function LiaIdCard() {
  const session = useLia((s) => s.session);
  const me = useLia((s) => s.me);
  const online = useLia((s) => s.online);
  const [security, setSecurity] = useState(false);

  if (!session) return <SignIn />;
  if (me && !me.emailVerified) return <Verify />;
  return (
    <div className="card pad lia-id">
      <div className="lia-logo">
        <Users size={18} />
      </div>
      <div className="grow" style={{ minWidth: 0 }}>
        <b>{t("acc.lia")}</b>
        <div className="faint ellipsis" style={{ fontSize: 12.5 }}>
          {t("lia.signed_in_as", { name: session.name })}
          {me?.email && ` · ${me.email}`} · {online ? t("lia.connected") : t("lia.connecting")}
        </div>
      </div>
      <button className="btn sm" onClick={() => setSecurity(true)}>
        <ShieldCheck size={14} />
        {t("lia.security")}
      </button>
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
      {security && <Security onClose={() => setSecurity(false)} />}
    </div>
  );
}

/** Безпека акаунта: пошта, пароль, пристрої, видалення. */
function Security({ onClose }: { onClose: () => void }) {
  const me = useLia((s) => s.me);
  const [devices, setDevices] = useState<LiaDevice[] | null>(null);
  const [tick, setTick] = useState(0);
  const [old, setOld] = useState("");
  const [p1, setP1] = useState("");
  const [p2, setP2] = useState("");
  const [email, setEmail] = useState("");
  const [emailPass, setEmailPass] = useState("");
  const [emailCode, setEmailCode] = useState<string | null>(null);
  const [delPass, setDelPass] = useState("");
  const [busy, run] = useBusy();

  useEffect(() => {
    api.lia<LiaDevice[]>("GET", "/api/sessions").then(setDevices).catch(fail);
  }, [tick]);

  return (
    <Modal title={t("lia.security")} onClose={onClose} size="wide" icon={<ShieldCheck size={18} />}>
      <section className="lia-sec">
        <h3>
          <MonitorSmartphone size={16} />
          {t("lia.devices")}
        </h3>
        <p className="hint">{t("lia.devices_text")}</p>
        {devices?.map((d) => (
          <div key={d.id} className="lia-device">
            <div className="grow" style={{ minWidth: 0 }}>
              <b className="ellipsis">{d.device}</b>
              <div className="faint" style={{ fontSize: 12 }}>
                {d.current ? t("lia.this_device") : t("lia.last_used", { when: ago(d.lastUsed) })}
              </div>
            </div>
            {!d.current && (
              <button className="btn ghost sm" disabled={busy} onClick={() => run(() => api.lia("DELETE", `/api/sessions/${d.id}`).then(() => setTick((x) => x + 1)))}>
                {t("lia.sign_out_device")}
              </button>
            )}
          </div>
        ))}
        {devices && devices.length > 1 && (
          <button
            className="btn sm"
            style={{ alignSelf: "flex-start" }}
            disabled={busy}
            onClick={() => run(() => api.lia("DELETE", "/api/sessions/others").then(() => (setTick((x) => x + 1), toast("success", t("lia.others_out")))))}
          >
            {t("lia.sign_out_others")}
          </button>
        )}
      </section>

      <section className="lia-sec">
        <h3>
          <KeyRound size={16} />
          {t("lia.password")}
        </h3>
        <div className="lia-form">
          <input className="input" type="password" placeholder={t("lia.pass_old")} value={old} maxLength={128} onChange={(e) => setOld(e.target.value)} />
          <input className="input" type="password" placeholder={t("acc.lia_pass_new")} value={p1} maxLength={128} onChange={(e) => setP1(e.target.value)} />
          <input className="input" type="password" placeholder={t("lia.pass_repeat")} value={p2} maxLength={128} onChange={(e) => setP2(e.target.value)} />
          {p2.length > 0 && p1 !== p2 && <p className="error-text">{t("lia.pass_mismatch")}</p>}
          <button
            className="btn"
            disabled={!old || p1.length < 8 || p1 !== p2 || busy}
            onClick={() =>
              run(async () => {
                await api.lia("POST", "/api/me/password", { old, new: p1 });
                setOld("");
                setP1("");
                setP2("");
                setTick((x) => x + 1);
                toast("success", t("lia.pass_changed"));
              })
            }
          >
            {t("lia.change_password")}
          </button>
        </div>
      </section>

      <section className="lia-sec">
        <h3>
          <Mail size={16} />
          {t("lia.email")}
        </h3>
        <p className="hint">{t("lia.email_now", { email: me?.email ?? "—" })}</p>
        {emailCode === null ? (
          <div className="lia-form">
            <input className="input" type="email" placeholder={t("lia.email_new")} value={email} maxLength={254} onChange={(e) => setEmail(e.target.value)} />
            <input className="input" type="password" placeholder={t("acc.lia_pass")} value={emailPass} maxLength={128} onChange={(e) => setEmailPass(e.target.value)} />
            <button
              className="btn"
              disabled={!EMAIL.test(email) || !emailPass || busy}
              onClick={() =>
                run(async () => {
                  await api.lia("POST", "/api/email/change", { email, password: emailPass });
                  setEmailPass("");
                  if (me?.mailEnabled) {
                    setEmailCode("");
                    toast("info", t("lia.code_sent"), t("lia.code_sent_to", { email }));
                  } else {
                    await refreshLia();
                  }
                })
              }
            >
              {t("lia.change_email")}
            </button>
          </div>
        ) : (
          <div className="lia-form">
            <input className="input mono" inputMode="numeric" placeholder={t("lia.code")} value={emailCode} maxLength={6} onChange={(e) => setEmailCode(e.target.value.replace(/\D/g, ""))} />
            <button
              className="btn primary"
              disabled={!/^\d{6}$/.test(emailCode) || busy}
              onClick={() =>
                run(async () => {
                  await api.lia("POST", "/api/email/verify", { code: emailCode });
                  setEmailCode(null);
                  setEmail("");
                  toast("success", t("lia.email_changed"));
                  await refreshLia();
                })
              }
            >
              {t("lia.confirm")}
            </button>
          </div>
        )}
      </section>

      <section className="lia-sec danger">
        <h3>
          <Trash2 size={16} />
          {t("lia.delete")}
        </h3>
        <p className="hint">{t("lia.delete_text")}</p>
        <div className="lia-form">
          <input className="input" type="password" placeholder={t("acc.lia_pass")} value={delPass} maxLength={128} onChange={(e) => setDelPass(e.target.value)} />
          <button
            className="btn danger"
            disabled={!delPass || busy}
            onClick={async () => {
              if (!(await confirmDialog({ title: t("lia.delete_q"), text: t("lia.delete_text"), danger: true }))) return;
              const ok = await run(() => api.lia("DELETE", "/api/me", { password: delPass }));
              if (ok) {
                toast("success", t("lia.deleted"));
                onClose();
                refreshLia();
              }
            }}
          >
            {t("lia.delete")}
          </button>
        </div>
      </section>
    </Modal>
  );
}

/**
 * «Грати з друзями» — лише для ліцензії: офлайн-нік з ЛІЯ ID і так входить через ЛІЮ.
 * Ліцензія в цьому запуску входить через ЛІЮ, тож у її LAN-світ зайдуть і пірати.
 */
export function FriendsPlayButton({ inst }: { inst: Instance }) {
  const ready = useLia((s) => !!s.session && !!s.me?.emailVerified);
  const running = useStore((s) => !!s.running[inst.id]);
  const busy = useStore((s) => Object.values(s.tasks).some((x) => x.tag === inst.id && x.state === "running"));
  const ms = useStore((s) => s.accounts.accounts.find((a) => a.id === s.accounts.selected)?.kind === "microsoft");
  if (!ready || !ms || running || busy) return null;
  return (
    <button className="btn friends-play" title={t("play.friends_hint")} onClick={() => launch(inst.id, undefined, undefined, true)}>
      <Users size={16} />
      {t("play.friends")}
    </button>
  );
}
