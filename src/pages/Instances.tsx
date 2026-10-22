import { useMemo, useState } from "react";
import { ArrowRightLeft } from "lucide-react";
import { MigrateModal } from "../components/Migrate";
import { open } from "@tauri-apps/plugin-dialog";
import { Boxes, FileDown, Plus, Search } from "lucide-react";
import { api } from "../lib/api";
import { t } from "../lib/i18n";
import { fail, navigate, setState, toast, useStore } from "../lib/store";
import { InstanceCard } from "../components/InstanceCard";
import { Empty } from "../components/ui";

export async function importMrpack() {
  const path = await open({ multiple: false, filters: [{ name: t("instances.mrpack_filter"), extensions: ["mrpack"] }] });
  if (typeof path !== "string") return;
  try {
    const inst = await api.importMrpack(path);
    toast("success", t("instances.imported", { name: inst.name }));
    navigate({ page: "instance", id: inst.id });
  } catch (e) {
    fail(e);
  }
}

export function Instances() {
  const instances = useStore((s) => s.instances);
  const [q, setQ] = useState("");
  const [migrate, setMigrate] = useState(false);
  const shown = useMemo(() => {
    const s = q.trim().toLowerCase();
    return s ? instances.filter((i) => `${i.name} ${i.mcVersion} ${i.loader}`.toLowerCase().includes(s)) : instances;
  }, [q, instances]);

  return (
    <div className="page">
      <div className="page-head">
        <h1>{t("instances.title")}</h1>
        {instances.length > 0 && (
          <div className="input-wrap" style={{ width: 240 }}>
            <Search size={16} />
            <input className="input" placeholder={t("instances.search")} value={q} onChange={(e) => setQ(e.target.value)} />
          </div>
        )}
        <button className="btn" onClick={() => setMigrate(true)}>
          <ArrowRightLeft size={16} />
          {t("mig.button")}
        </button>
        <button className="btn" onClick={importMrpack}>
          <FileDown size={16} />
          {t("instances.import")}
        </button>
        <button className="btn primary" onClick={() => setState({ newInstanceOpen: true })}>
          <Plus size={16} />
          {t("instances.new")}
        </button>
      </div>
      {instances.length === 0 ? (
        <div className="card">
          <Empty icon={<Boxes size={40} className="faint" />} title={t("instances.empty_title")} text={t("instances.empty_text")}>
            <div className="row">
              <button className="btn primary" onClick={() => setState({ newInstanceOpen: true })}>
                <Plus size={16} />
                {t("instances.new")}
              </button>
              <button className="btn" onClick={() => setMigrate(true)}>
                <ArrowRightLeft size={16} />
                {t("mig.button")}
              </button>
            </div>
          </Empty>
        </div>
      ) : shown.length === 0 ? (
        <Empty title={t("instances.no_match")} />
      ) : (
        <div className="igrid">
          {shown.map((i) => (
            <InstanceCard key={i.id} inst={i} />
          ))}
        </div>
      )}
      {migrate && <MigrateModal onClose={() => setMigrate(false)} />}
    </div>
  );
}
