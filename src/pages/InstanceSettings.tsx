import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ImagePlus, Trash2, Wrench } from "lucide-react";
import { api, type Instance, type Loader } from "../lib/api";
import { gb } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, navigate, toast, useStore } from "../lib/store";
import { deleteInstance } from "../components/InstanceCard";
import { IconPicker, VersionPicker } from "../components/NewInstance";
import { Slider, Switch } from "../components/ui";

async function patch(id: string, p: Record<string, unknown>) {
  try {
    await api.instanceUpdate(id, p);
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

export function InstanceSettings({ inst }: { inst: Instance }) {
  const settings = useStore((s) => s.settings);
  const info = useStore((s) => s.info);
  const running = useStore((s) => !!s.running[inst.id]);
  const [name, setName] = useState(inst.name);
  const [sel, setSel] = useState({ mc: inst.mcVersion as string | null, loader: inst.loader as Loader, loaderVersion: inst.loaderVersion ?? null });
  const [javaPath, setJavaPath] = useState(inst.javaPath ?? "");
  const [jvm, setJvm] = useState(inst.jvmArgs ?? "");
  const [repairing, setRepairing] = useState(false);

  useEffect(() => setName(inst.name), [inst.name]);

  const changed = sel.mc !== inst.mcVersion || sel.loader !== inst.loader || (sel.loader !== "vanilla" && sel.loaderVersion !== (inst.loaderVersion ?? null));
  const total = info?.totalMemoryMb ?? 8192;
  const maxMem = Math.max(2048, Math.floor((total - 1024) / 512) * 512);
  const memory = inst.memoryMb ?? settings?.memoryMb ?? 2048;

  return (
    <div className="settings-cols">
      <section className="card pad set-card">
        <h3>{t("iset.general")}</h3>
        <div className="field">
          <label>{t("iset.name")}</label>
          <input
            className="input"
            value={name}
            maxLength={48}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => name.trim() && name !== inst.name && patch(inst.id, { name })}
            onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
          />
        </div>
        <div className="field">
          <label>{t("iset.icon")}</label>
          <IconPicker value={inst.icon} onChange={(icon) => patch(inst.id, { icon })} />
          <button
            className="btn sm"
            style={{ alignSelf: "flex-start" }}
            onClick={async () => {
              const p = await open({ multiple: false, filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp"] }] });
              if (typeof p === "string") api.instanceSetIconFile(inst.id, p).catch(fail);
            }}
          >
            <ImagePlus size={15} />
            {t("iset.icon_custom")}
          </button>
        </div>
      </section>

      <section className="card pad set-card">
        <h3>{t("iset.version")}</h3>
        <VersionPicker mc={sel.mc} loader={sel.loader} loaderVersion={sel.loaderVersion} onChange={setSel} />
        <p className="hint">{t("iset.version_hint")}</p>
        <button
          className="btn primary"
          style={{ alignSelf: "flex-start" }}
          disabled={!changed || !sel.mc || running || (sel.loader !== "vanilla" && !sel.loaderVersion)}
          onClick={async () => {
            if (await patch(inst.id, { mcVersion: sel.mc, loader: sel.loader, loaderVersion: sel.loader === "vanilla" ? null : sel.loaderVersion })) toast("success", t("iset.saved"));
          }}
        >
          {t("iset.apply")}
        </button>
      </section>

      <section className="card pad set-card">
        <h3>{t("iset.memory")}</h3>
        <label className="row" style={{ gap: 10 }}>
          <Switch on={inst.memoryMb == null} onChange={(v) => patch(inst.id, { memoryMb: v ? null : memory })} />
          {t("iset.use_global")}
        </label>
        {inst.memoryMb != null && (
          <div className="mem">
            <Slider value={memory} min={1024} max={maxMem} step={512} onChange={(v) => patch(inst.id, { memoryMb: v })} />
            <b className="mono">{gb(memory)}</b>
          </div>
        )}

        <h3 style={{ marginTop: 10 }}>{t("iset.java")}</h3>
        <div className="row">
          <input
            className="input mono"
            style={{ fontSize: 12.5 }}
            placeholder={t("iset.java_auto")}
            value={javaPath}
            onChange={(e) => setJavaPath(e.target.value)}
            onBlur={() => javaPath !== (inst.javaPath ?? "") && patch(inst.id, { javaPath: javaPath.trim() || null })}
          />
          <button
            className="btn"
            onClick={async () => {
              const p = await open({ multiple: false, filters: [{ name: "Java", extensions: ["exe"] }] });
              if (typeof p === "string") {
                setJavaPath(p);
                patch(inst.id, { javaPath: p });
              }
            }}
          >
            {t("set.change")}
          </button>
        </div>

        <h3 style={{ marginTop: 10 }}>{t("iset.jvm_args")}</h3>
        <textarea
          className="textarea"
          rows={2}
          placeholder={settings?.jvmArgs || "-XX:+UseG1GC …"}
          value={jvm}
          onChange={(e) => setJvm(e.target.value)}
          onBlur={() => jvm !== (inst.jvmArgs ?? "") && patch(inst.id, { jvmArgs: jvm.trim() || null })}
        />
      </section>

      <section className="card pad set-card">
        <h3>{t("iset.window")}</h3>
        <div className="row">
          <input
            className="input"
            type="number"
            placeholder={String(settings?.width ?? 1280)}
            defaultValue={inst.width ?? ""}
            onBlur={(e) => patch(inst.id, { width: e.target.value ? Number(e.target.value) : null })}
          />
          <span className="faint">×</span>
          <input
            className="input"
            type="number"
            placeholder={String(settings?.height ?? 720)}
            defaultValue={inst.height ?? ""}
            onBlur={(e) => patch(inst.id, { height: e.target.value ? Number(e.target.value) : null })}
          />
        </div>
        <label className="row" style={{ gap: 10 }}>
          <Switch on={inst.fullscreen ?? settings?.fullscreen ?? false} onChange={(v) => patch(inst.id, { fullscreen: v })} />
          {t("set.fullscreen")}
        </label>

        <h3 style={{ marginTop: 10 }}>{t("iset.danger")}</h3>
        <div className="danger-row">
          <div className="grow">
            <b>{t("instances.repair")}</b>
            <p className="hint">{t("iset.repair_text")}</p>
          </div>
          <button
            className="btn"
            disabled={repairing || running}
            onClick={async () => {
              setRepairing(true);
              try {
                await api.repair(inst.id);
                toast("success", t("instances.repair_done"));
              } catch (e) {
                fail(e);
              } finally {
                setRepairing(false);
              }
            }}
          >
            {repairing ? <div className="spinner" style={{ width: 14, height: 14 }} /> : <Wrench size={15} />}
            {t("instances.repair")}
          </button>
        </div>
        <div className="danger-row">
          <div className="grow">
            <b>{t("common.delete")}</b>
            <p className="hint">{t("iset.delete_text")}</p>
          </div>
          <button
            className="btn danger"
            disabled={running}
            onClick={async () => {
              if (await deleteInstance(inst)) navigate({ page: "instances" });
            }}
          >
            <Trash2 size={15} />
            {t("common.delete")}
          </button>
        </div>
      </section>
    </div>
  );
}
