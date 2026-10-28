import { useMemo } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { ArrowUpCircle, RefreshCw } from "lucide-react";
import { api } from "../lib/api";
import { bytes } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, setState, useStore } from "../lib/store";
import { checkForUpdate, installUpdate } from "../lib/updater";
import { Modal, Progress } from "./ui";

/** Кнопка в заголовку вікна, коли є нова версія. */
export function UpdatePill() {
  const st = useStore((s) => s.update);
  if (st.status !== "available" && st.status !== "downloading" && st.status !== "installing") return null;
  return (
    <button className="update-pill" onClick={() => setState({ updateOpen: true })}>
      <ArrowUpCircle size={15} />
      {t("upd.pill", { v: st.update.version })}
    </button>
  );
}

export function UpdateModal() {
  const st = useStore((s) => s.update);
  const running = useStore((s) => Object.keys(s.running).length > 0);
  const version = useStore((s) => s.info?.version ?? "");
  const close = () => setState({ updateOpen: false });
  const notes = useMemo(() => {
    if (!("update" in st)) return "";
    const body = st.update.body?.trim() || t("upd.no_notes");
    return DOMPurify.sanitize(marked.parse(body, { async: false }) as string, { FORBID_TAGS: ["style", "iframe", "form", "script"], FORBID_ATTR: ["style"] });
  }, [st]);
  if (!("update" in st)) return null;
  const busy = st.status === "downloading" || st.status === "installing";
  const pct = st.status === "downloading" && st.total > 0 ? (st.done / st.total) * 100 : 0;

  return (
    <Modal
      title={t("upd.title", { v: st.update.version })}
      icon={<ArrowUpCircle size={22} color="var(--yellow)" />}
      onClose={busy ? () => {} : close}
      foot={
        <>
          {!busy && (
            <button className="btn ghost" onClick={close}>
              {t("upd.later")}
            </button>
          )}
          <button className="btn primary" disabled={busy} onClick={installUpdate}>
            {busy ? <div className="spinner" style={{ width: 14, height: 14 }} /> : <RefreshCw size={15} />}
            {t("upd.install")}
          </button>
        </>
      }
    >
      <p className="faint">{t("upd.current", { v: version })}</p>
      <div
        className="prose selectable update-notes"
        dangerouslySetInnerHTML={{ __html: notes }}
        onClick={(e) => {
          // Посилання відкриваємо в браузері, бо інакше вікно лаунчера перейде на чужу сторінку без дороги назад.
          const a = (e.target as HTMLElement).closest("a");
          if (a) {
            e.preventDefault();
            if (a.href) api.openUrl(a.href).catch(fail);
          }
        }}
      />
      {running && <p className="hint">{t("upd.game_note")}</p>}
      {st.status === "downloading" && (
        <div className="field">
          <Progress value={pct} indeterminate={st.total === 0} />
          <small className="faint">{t("upd.downloading", { p: st.total > 0 ? `${bytes(st.done)} / ${bytes(st.total)}` : bytes(st.done) })}</small>
        </div>
      )}
      {st.status === "installing" && <p className="faint">{t("upd.installing")}</p>}
    </Modal>
  );
}

/** Рядок у налаштуваннях: версія, стан і ручна перевірка. */
export function UpdateRow() {
  const st = useStore((s) => s.update);
  return (
    <div className="row" style={{ flexWrap: "wrap" }}>
      {"update" in st ? (
        <button className="btn sm primary" onClick={() => setState({ updateOpen: true })}>
          <ArrowUpCircle size={14} />
          {t("upd.pill", { v: st.update.version })}
        </button>
      ) : (
        <button className="btn sm" disabled={st.status === "checking"} onClick={() => checkForUpdate(true)}>
          <RefreshCw size={14} className={st.status === "checking" ? "spin" : ""} />
          {st.status === "checking" ? t("upd.checking") : t("upd.check")}
        </button>
      )}
      {st.status === "none" && <span className="faint" style={{ fontSize: 12.5 }}>{t("upd.none")}</span>}
    </div>
  );
}
