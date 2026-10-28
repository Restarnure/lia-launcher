import { useEffect, useMemo, useState } from "react";
import { Check as CheckIcon, ChevronRight, Minus, Share2 } from "lucide-react";
import { api, type Instance, type LiaPack, type PackSelection, type PublishPlan } from "../lib/api";
import { bytes } from "../lib/format";
import { t, type Key } from "../lib/i18n";
import { useLia } from "../lib/lia";
import { fail, refreshInstances, toast } from "../lib/store";
import { Check, Modal, Seg } from "./ui";

// До 1.1.5 зв'язок «моя збірка → опублікована» жив лише тут. Тепер він у теці збірки, а це для переходу.
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

// ------------------------------------------------------------ вибір файлів (як Selection у packs.rs)

const under = (rel: string, dir: string) => rel === dir || rel.startsWith(dir + "/");

function picks(sel: PackSelection, rel: string): boolean {
  const deepest = (l: string[]) => Math.max(-1, ...l.filter((p) => under(rel, p)).map((p) => p.length));
  const i = deepest(sel.include);
  return i >= 0 && i > deepest(sel.exclude);
}

/** Увімкнути чи вимкнути шлях з усім, що в ньому. Записи всередині стають зайвими. */
function toggle(sel: PackSelection, path: string, on: boolean): PackSelection {
  const next = { include: sel.include.filter((p) => !under(p, path)), exclude: sel.exclude.filter((p) => !under(p, path)) };
  if (picks(next, path) !== on) (on ? next.include : next.exclude).push(path);
  return next;
}

type Kind = PublishPlan["files"][number]["kind"];
interface Node {
  name: string;
  path: string;
  size: number;
  kind?: Kind;
  children: Node[];
}

function buildTree(files: PublishPlan["files"]): Node[] {
  const root: Node = { name: "", path: "", size: 0, children: [] };
  for (const f of files) {
    let cur = root;
    const parts = f.path.split("/");
    parts.forEach((name, i) => {
      cur.size += f.size;
      const path = parts.slice(0, i + 1).join("/");
      let next = cur.children.find((c) => c.name === name);
      if (!next) {
        next = { name, path, size: 0, children: [] };
        cur.children.push(next);
      }
      cur = next;
    });
    cur.size += f.size;
    cur.kind = f.kind;
  }
  // Спершу теки, потім файли, за абеткою.
  const sort = (n: Node) => {
    n.children.sort((a, b) => (a.kind ? 1 : 0) - (b.kind ? 1 : 0) || a.name.localeCompare(b.name));
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}

/** Скільки файлів теки вибрано: усі, жодного чи частина. */
function state(n: Node, sel: PackSelection): "on" | "off" | "some" {
  if (n.kind) return picks(sel, n.path) ? "on" : "off";
  let on = 0;
  let off = 0;
  const walk = (x: Node) => {
    if (x.kind) picks(sel, x.path) ? on++ : off++;
    else x.children.forEach(walk);
  };
  walk(n);
  return off === 0 ? "on" : on === 0 ? "off" : "some";
}

const KIND_LABEL: Record<Kind, Key> = { base: "lia.kind_base", link: "lia.kind_link", upload: "lia.kind_upload", stored: "lia.kind_stored", big: "lia.kind_big" };

function Row({ n, depth, sel, setSel, open, setOpen }: { n: Node; depth: number; sel: PackSelection; setSel: (s: PackSelection) => void; open: Set<string>; setOpen: (s: Set<string>) => void }) {
  const st = state(n, sel);
  const expanded = open.has(n.path);
  return (
    <>
      <div className="pack-row" style={{ paddingLeft: 6 + depth * 18 }}>
        {n.kind ? (
          <span className="pack-chevron" />
        ) : (
          <button
            className={`pack-chevron ${expanded ? "open" : ""}`}
            aria-label={n.name}
            onClick={() => {
              const s = new Set(open);
              if (expanded) s.delete(n.path);
              else s.add(n.path);
              setOpen(s);
            }}
          >
            <ChevronRight size={14} />
          </button>
        )}
        <button className={`check pack-check ${st}`} role="checkbox" aria-checked={st === "some" ? "mixed" : st === "on"} onClick={() => setSel(toggle(sel, n.path, st !== "on"))}>
          <span className="box">{st === "on" ? <CheckIcon size={12} strokeWidth={3.5} /> : st === "some" ? <Minus size={12} strokeWidth={3.5} /> : null}</span>
          <span className="grow ellipsis">{n.name}</span>
        </button>
        {n.kind && <span className={`pack-kind ${n.kind}`}>{t(KIND_LABEL[n.kind])}</span>}
        <span className="faint pack-size">{bytes(n.size)}</span>
      </div>
      {expanded && n.children.map((c) => <Row key={c.path} n={c} depth={depth + 1} sel={sel} setSel={setSel} open={open} setOpen={setOpen} />)}
    </>
  );
}

function PublishModal({ inst, onClose }: { inst: Instance; onClose: () => void }) {
  const [plan, setPlan] = useState<PublishPlan | null>(null);
  const [sel, setSel] = useState<PackSelection>({ include: [], exclude: [] });
  const [shareOptions, setShareOptions] = useState(true);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [existing, setExisting] = useState<LiaPack | null>(null);
  const [checking, setChecking] = useState(true);
  const [visibility, setVisibility] = useState<"friends" | "public">("friends");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .liaPublishPlan(inst.id)
      .then(async (p) => {
        setPlan(p);
        setSel(p.selection);
        setShareOptions(p.shareOptions);
        const id = p.packId ?? published()[inst.id];
        if (!id) return;
        // Видалену на сервері збірку публікуємо заново.
        const pack = await api.lia<LiaPack>("GET", `/api/packs/${id}`).catch(() => null);
        if (pack) {
          setExisting(pack);
          setVisibility(pack.visibility);
          setDescription(pack.description);
        }
      })
      .catch((e) => {
        fail(e);
        onClose();
      })
      .finally(() => setChecking(false));
  }, [inst.id]);

  const tree = useMemo(() => (plan ? buildTree(plan.files) : []), [plan]);
  const sum = useMemo(() => {
    let upload = 0;
    let total = 0;
    const big: string[] = [];
    for (const f of plan?.files ?? []) {
      if (!picks(sel, f.path)) continue;
      total += f.size;
      if (f.kind === "upload") upload += f.size;
      if (f.kind === "big") big.push(f.path);
    }
    return { upload, total, big };
  }, [plan, sel]);

  const go = async () => {
    setBusy(true);
    try {
      const p = await api.liaPublish(inst.id, visibility, description, existing?.id ?? null, sel, shareOptions);
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
      onClose={busy ? () => {} : onClose}
      size="wide"
      foot={
        <>
          <button className="btn ghost" disabled={busy} onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" disabled={busy || checking || sum.big.length > 0} onClick={go}>
            {busy && <div className="spinner" style={{ width: 14, height: 14 }} />}
            {existing ? t("lia.publish_new_version", { v: existing.version + 1 }) : t("lia.publish")}
          </button>
        </>
      }
    >
      <p className="muted">{t("lia.publish_text")}</p>
      <div className="row" style={{ gap: 16, alignItems: "flex-start", flexWrap: "wrap" }}>
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
        <div className="field grow">
          <label>{t("lia.description")}</label>
          <textarea className="textarea" rows={2} maxLength={500} value={description} onChange={(e) => setDescription(e.target.value)} />
        </div>
      </div>
      <div className="field">
        <label>{t("lia.contents")}</label>
        {!plan ? (
          <div className="pack-tree pack-loading">
            <div className="row faint">
              <div className="spinner" style={{ width: 16, height: 16 }} />
              {t("lia.publish_scanning")}
            </div>
            <small className="faint">{t("lia.publish_scanning_hint")}</small>
          </div>
        ) : (
          <>
            {plan.base && <small className="faint">{t("lia.base", { name: plan.base.name, v: plan.base.versionName })}</small>}
            <div className="pack-tree">
              {tree.map((n) => (
                <Row key={n.path} n={n} depth={0} sel={sel} setSel={setSel} open={open} setOpen={setOpen} />
              ))}
            </div>
            <small className={sum.upload > 0 ? "" : "faint"}>
              {sum.upload > 0 ? t("lia.upload_size", { size: bytes(sum.upload) }) : t("lia.upload_none")} · {t("lia.player_gets", { size: bytes(sum.total) })}
            </small>
            {sum.big.length > 0 && <p className="hint" style={{ color: "var(--red)" }}>{t("lia.too_big", { files: sum.big.join(", ") })}</p>}
          </>
        )}
      </div>
      {plan?.options && (
        <div className="field">
          <Check on={shareOptions} onChange={setShareOptions}>
            {t("lia.share_options")}
          </Check>
          <small className="faint">{t("lia.share_options_hint")}</small>
        </div>
      )}
    </Modal>
  );
}
