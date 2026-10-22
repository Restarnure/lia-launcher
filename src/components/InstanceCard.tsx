import { Copy, FolderOpen, Settings2, Trash2, Wrench } from "lucide-react";
import { api, type Instance } from "../lib/api";
import { ago, duration } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, navigate, refreshInstances, toast, useStore } from "../lib/store";
import { confirmDialog, InstanceIcon, LoaderBadge, Menu, PlayButton } from "./ui";

export async function deleteInstance(inst: Instance) {
  const ok = await confirmDialog({ title: t("instances.delete_title", { name: inst.name }), text: t("instances.delete_text"), danger: true });
  if (!ok) return false;
  try {
    await api.instanceDelete(inst.id);
    await refreshInstances();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

export function instanceMenu(inst: Instance) {
  return [
    { label: t("inst.tab.settings"), icon: <Settings2 size={16} />, onClick: () => navigate({ page: "instance", id: inst.id, tab: "settings" }) },
    { label: t("common.open_folder"), icon: <FolderOpen size={16} />, onClick: () => api.instanceFolder(inst.id).then((p) => api.openPath(p)).catch(fail) },
    {
      label: t("instances.duplicate"),
      icon: <Copy size={16} />,
      onClick: () =>
        api
          .instanceDuplicate(inst.id, t("instances.copy_suffix"))
          .then(() => refreshInstances())
          .catch(fail),
    },
    {
      label: t("instances.repair"),
      icon: <Wrench size={16} />,
      onClick: () =>
        api
          .repair(inst.id)
          .then(() => toast("success", t("instances.repair_done")))
          .catch(fail),
    },
    { label: t("common.delete"), icon: <Trash2 size={16} />, danger: true, separator: true, onClick: () => void deleteInstance(inst) },
  ];
}

export function InstanceCard({ inst }: { inst: Instance }) {
  const running = useStore((s) => !!s.running[inst.id]);
  return (
    <div className={`icard ${running ? "live" : ""}`} onClick={() => navigate({ page: "instance", id: inst.id })}>
      <div className="icard-art">
        <InstanceIcon inst={inst} size={76} />
        {running && (
          <span className="icard-live">
            <span className="dot" />
            {t("common.playing")}
          </span>
        )}
      </div>
      <div className="icard-body">
        <b className="ellipsis">{inst.name}</b>
        <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
          <span className="badge">{inst.mcVersion}</span>
          <LoaderBadge loader={inst.loader} />
        </div>
        <small className="faint ellipsis">
          {inst.lastPlayed ? t("instances.last", { when: ago(inst.lastPlayed) }) : t("instances.never")}
          {inst.playTime > 60 ? ` · ${duration(inst.playTime)}` : ""}
        </small>
      </div>
      <div className="icard-actions" onClick={(e) => e.stopPropagation()}>
        <PlayButton inst={inst} size="sm" />
        <Menu items={instanceMenu(inst)} className="btn icon sm" />
      </div>
    </div>
  );
}
