import { useState } from "react";
import { LogIn, Users } from "lucide-react";
import { api } from "../lib/api";
import { t } from "../lib/i18n";
import { fail, navigate, refreshAccounts, toast, useStore } from "../lib/store";
import { Modal } from "./ui";

export function AddAccount({ onClose }: { onClose: () => void }) {
  const msAvailable = useStore((s) => s.accounts.msAvailable);
  const [nick, setNick] = useState("");
  const [busy, setBusy] = useState<"ms" | "offline" | "lia" | null>(null);
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(nick);
  const [liaName, setLiaName] = useState("");
  const [liaPass, setLiaPass] = useState("");
  const [liaNew, setLiaNew] = useState(false);
  const liaValid = /^[A-Za-z0-9_]{3,16}$/.test(liaName) && liaPass.length >= (liaNew ? 8 : 1);

  const liaLogin = async () => {
    setBusy("lia");
    try {
      const a = await api.loginLia(liaName, liaPass, liaNew);
      await refreshAccounts();
      toast("success", t("acc.added", { name: a.name }));
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  };

  const offline = async () => {
    setBusy("offline");
    try {
      const a = await api.addOffline(nick);
      await refreshAccounts();
      toast("success", t("acc.added", { name: a.name }));
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  };

  const microsoft = async () => {
    setBusy("ms");
    try {
      const a = await api.loginMicrosoft();
      await refreshAccounts();
      toast("success", t("acc.added", { name: a.name }));
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal title={t("acc.add")} onClose={onClose}>
      <div className="add-acc ms">
        <div className="row" style={{ alignItems: "flex-start", gap: 12 }}>
          <div className="ms-logo">
            <span />
            <span />
            <span />
            <span />
          </div>
          <div className="grow">
            <b>{t("acc.microsoft")}</b>
            <p className="hint">{t("acc.microsoft_desc")}</p>
          </div>
        </div>
        {msAvailable ? (
          <button className="btn blue" disabled={busy !== null} onClick={microsoft}>
            {busy === "ms" ? <div className="spinner" style={{ width: 14, height: 14 }} /> : <LogIn size={16} />}
            {busy === "ms" ? t("acc.login_wait") : t("acc.login_ms")}
          </button>
        ) : !import.meta.env.DEV ? (
          <p className="hint">{t("acc.ms_soon")}</p>
        ) : (
          <p className="hint">
            {t("acc.ms_unavailable")}{" "}
            <a
              href="#"
              onClick={(e) => {
                e.preventDefault();
                onClose();
                navigate({ page: "settings" });
              }}
            >
              {t("acc.ms_setup")}
            </a>
          </p>
        )}
      </div>
      <div className="add-acc lia">
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
          <input className="input" placeholder={t("acc.lia_nick")} value={liaName} maxLength={16} onChange={(e) => setLiaName(e.target.value.replace(/[^A-Za-z0-9_]/g, ""))} />
          <input
            className="input"
            type="password"
            placeholder={liaNew ? t("acc.lia_pass_new") : t("acc.lia_pass")}
            value={liaPass}
            maxLength={128}
            onChange={(e) => setLiaPass(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && liaValid && liaLogin()}
          />
          <button className="btn primary" disabled={!liaValid || busy !== null} onClick={liaLogin}>
            {busy === "lia" && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {liaNew ? t("acc.lia_register") : t("acc.lia_login")}
          </button>
        </div>
        <a
          href="#"
          className="hint"
          onClick={(e) => {
            e.preventDefault();
            setLiaNew((v) => !v);
          }}
        >
          {liaNew ? t("acc.lia_have") : t("acc.lia_new")}
        </a>
      </div>
      <div className="add-acc">
        <div>
          <b>{t("acc.offline")}</b>
          <p className="hint">{t("acc.offline_desc")}</p>
        </div>
        <div className="row">
          <input
            className="input"
            placeholder="Steve"
            value={nick}
            maxLength={16}
            autoFocus
            onChange={(e) => setNick(e.target.value.replace(/[^A-Za-z0-9_]/g, ""))}
            onKeyDown={(e) => e.key === "Enter" && valid && offline()}
          />
          <button className="btn primary" disabled={!valid || busy !== null} onClick={offline}>
            {t("acc.add")}
          </button>
        </div>
        {nick.length > 0 && !valid && <p className="error-text">{t("err.invalid_nick")}</p>}
      </div>
    </Modal>
  );
}
