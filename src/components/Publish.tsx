import { useEffect, useState } from "react";
import { Share2 } from "lucide-react";
import { api, type Instance, type LiaPack } from "../lib/api";
import { t } from "../lib/i18n";
import { useLia } from "../lib/lia";
import { fail, refreshInstances, toast } from "../lib/store";
import { Modal, Seg } from "./ui";

// ponytail: зв'язок «моя збірка → опублікована» зберігаємо локально; на іншому ПК публікація почнеться заново.
const KEY = "lia:published";
function published(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}

/** Опублікувати збірку друзям або випустити нову версію вже опублікованої. */
export function PublishButton({ inst }: { inst: Instance }) {
  const session = useLia((s) => (s.me?.emailVerified ? s.session : null));
  const [open, setOpen] = useState(false);
  // Збірку, встановлену від друга, публікує її автор, а не ми.
  if (!session || inst.modpack?.projectId.startsWith("lia:")) return null;
  return (
    <>
      <button className="btn" onClick={() => setOpen(true)}>
        <Share2 size={15} />
        {t("lia.share_pack")}
      </button>
      {open && <PublishModal inst={inst} onClose={() => setOpen(false)} />}
    </>
  );
}

function PublishModal({ inst, onClose }: { inst: Instance; onClose: () => void }) {
  const [packId, setPackId] = useState<string | null>(published()[inst.id] ?? null);
  const [existing, setExisting] = useState<LiaPack | null>(null);
  const [visibility, setVisibility] = useState<"friends" | "public">("friends");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!packId) return;
    api
      .lia<LiaPack>("GET", `/api/packs/${packId}`)
      .then((p) => {
        setExisting(p);
        setVisibility(p.visibility);
        setDescription(p.description);
      })
      .catch(() => setPackId(null)); // видалена на сервері — публікуємо заново
  }, [packId]);

  const go = async () => {
    setBusy(true);
    try {
      const p = await api.liaPublish(inst.id, visibility, description, existing?.id ?? null);
      localStorage.setItem(KEY, JSON.stringify({ ...published(), [inst.id]: p.id }));
      await refreshInstances();
      toast("success", existing ? t("lia.pack_updated", { v: p.version }) : t("lia.pack_published"));
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={existing ? t("lia.publish_update", { name: inst.name }) : t("lia.publish_title", { name: inst.name })}
      onClose={onClose}
      foot={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={busy} onClick={go}>
            {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {existing ? t("lia.publish_new_version", { v: existing.version + 1 }) : t("lia.publish")}
          </button>
        </>
      }
    >
      <p className="muted">{t("lia.publish_text")}</p>
      <div className="field">
        <label>{t("lia.visibility")}</label>
        <Seg
          value={visibility}
          onChange={setVisibility}
          options={[
            { value: "friends", label: t("lia.vis_friends") },
            { value: "public", label: t("lia.vis_public") },
          ]}
        />
      </div>
      <div className="field">
        <label>{t("lia.description")}</label>
        <textarea className="textarea" rows={3} maxLength={500} value={description} onChange={(e) => setDescription(e.target.value)} />
      </div>
    </Modal>
  );
}
