import { useState } from "react";
import { LogIn } from "lucide-react";
import { api } from "../lib/api";
import { t } from "../lib/i18n";
import { fail, navigate, refreshAccounts, toast, useStore } from "../lib/store";
import { Modal } from "./ui";

export function AddAccount({ onClose }: { onClose: () => void }) {
  const msAvailable = useStore((s) => s.accounts.msAvailable);
  const [nick, setNick] = useState("");
  const [busy, setBusy] = useState<"ms" | "offline" | null>(null);
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(nick);

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
