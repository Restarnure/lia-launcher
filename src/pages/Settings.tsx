import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, FolderOpen, RotateCcw, XCircle } from "lucide-react";
import { api, type Settings as S } from "../lib/api";
import { gb } from "../lib/format";
import { t } from "../lib/i18n";
import { fail, saveSettings, useStore } from "../lib/store";
import { Logo } from "../components/Shell";
import { Seg, Slider, Switch } from "../components/ui";

export function Settings() {
  const s = useStore((x) => x.settings);
  const info = useStore((x) => x.info);
  if (!s || !info) return null;
  return <SettingsBody s={s} total={info.totalMemoryMb} dataDir={info.dataDir} version={info.version} />;
}

function SettingsBody({ s, total, dataDir, version }: { s: S; total: number; dataDir: string; version: string }) {
  const [mem, setMem] = useState(s.memoryMb);
  const [jvm, setJvm] = useState(s.jvmArgs);
  const [w, setW] = useState(String(s.width));
  const [h, setH] = useState(String(s.height));
  const [javaMode, setJavaMode] = useState<"auto" | "custom">(s.javaPath ? "custom" : "auto");
  const [javaPath, setJavaPath] = useState(s.javaPath ?? "");
  const [javaTest, setJavaTest] = useState<{ ok: boolean; text: string } | null>(null);
  const [clientId, setClientId] = useState(s.msClientId);
  const [conc, setConc] = useState(s.concurrency);

  useEffect(() => setMem(s.memoryMb), [s.memoryMb]);

  const maxMem = Math.max(2048, Math.floor((total - 1024) / 512) * 512);
  const rec = total >= 15500 ? 4096 : total >= 7500 ? 3072 : 2048;

  const testJava = async (path: string) => {
    setJavaTest(null);
    try {
      const r = await api.javaProbe(path);
      setJavaTest({ ok: true, text: r.line });
    } catch (e) {
      setJavaTest({ ok: false, text: String((e as { message?: string }).message ?? e) });
    }
  };

  return (
    <div className="page">
      <div className="page-head">
        <h1>{t("set.title")}</h1>
      </div>
      <div className="settings-cols">
        <section className="card pad set-card">
          <h3>{t("set.general")}</h3>
          <div className="set-row">
            <span className="grow">{t("set.language")}</span>
            <Seg
              value={s.language}
              onChange={(language) => saveSettings({ language })}
              options={[
                { value: "uk", label: "Українська" },
                { value: "en", label: "English" },
              ]}
            />
          </div>
          <div className="set-row">
            <span className="grow">{t("set.on_launch")}</span>
            <Seg
              value={s.onLaunch}
              onChange={(onLaunch) => saveSettings({ onLaunch })}
              options={[
                { value: "keep", label: t("set.on_launch.keep") },
                { value: "minimize", label: t("set.on_launch.minimize") },
                { value: "close", label: t("set.on_launch.close") },
              ]}
            />
          </div>
          <div className="set-row">
            <span className="grow">{t("set.featured")}</span>
            <Switch on={s.showFeatured} onChange={(showFeatured) => saveSettings({ showFeatured })} />
          </div>
        </section>

        <section className="card pad set-card">
          <h3>{t("set.game")}</h3>
          <div className="field">
            <div className="row">
              <label className="label grow">{t("set.memory")}</label>
              <b className="mono">{gb(mem)}</b>
            </div>
            <Slider value={mem} min={1024} max={maxMem} step={512} onChange={setMem} />
            <p className="hint">{t("set.memory_hint", { rec: (rec / 1024).toFixed(0), total: (total / 1024).toFixed(0) })}</p>
            {mem !== s.memoryMb && (
              <button className="btn sm primary" style={{ alignSelf: "flex-start" }} onClick={() => saveSettings({ memoryMb: mem })}>
                {t("common.save")}
              </button>
            )}
          </div>
          <div className="field">
            <label>{t("set.resolution")}</label>
            <div className="row">
              <input className="input" type="number" value={w} onChange={(e) => setW(e.target.value)} onBlur={() => Number(w) !== s.width && saveSettings({ width: Number(w) || 1280 })} />
              <span className="faint">×</span>
              <input className="input" type="number" value={h} onChange={(e) => setH(e.target.value)} onBlur={() => Number(h) !== s.height && saveSettings({ height: Number(h) || 720 })} />
            </div>
          </div>
          <div className="set-row">
            <span className="grow">{t("set.fullscreen")}</span>
            <Switch on={s.fullscreen} onChange={(fullscreen) => saveSettings({ fullscreen })} />
          </div>
          <div className="field">
            <label>{t("set.jvm")}</label>
            <textarea className="textarea" rows={2} value={jvm} placeholder="-XX:+UseZGC" onChange={(e) => setJvm(e.target.value)} onBlur={() => jvm !== s.jvmArgs && saveSettings({ jvmArgs: jvm })} />
            <p className="hint">{t("set.jvm_hint")}</p>
          </div>
        </section>

        <section className="card pad set-card">
          <h3>{t("set.java")}</h3>
          <Seg
            value={javaMode}
            onChange={(m) => {
              setJavaMode(m);
              if (m === "auto") saveSettings({ javaPath: null });
            }}
            options={[
              { value: "auto", label: t("set.java_auto") },
              { value: "custom", label: t("set.java_custom") },
            ]}
          />
          {javaMode === "auto" ? (
            <p className="hint">{t("set.java_auto_desc")}</p>
          ) : (
            <>
              <div className="row">
                <input
                  className="input mono"
                  style={{ fontSize: 12.5 }}
                  value={javaPath}
                  placeholder="C:\Program Files\Java\jdk-21\bin\javaw.exe"
                  onChange={(e) => setJavaPath(e.target.value)}
                  onBlur={() => saveSettings({ javaPath: javaPath.trim() || null })}
                />
                <button
                  className="btn"
                  onClick={async () => {
                    const p = await open({ multiple: false, filters: [{ name: "Java", extensions: ["exe"] }] });
                    if (typeof p === "string") {
                      setJavaPath(p);
                      saveSettings({ javaPath: p });
                      testJava(p);
                    }
                  }}
                >
                  {t("set.java_pick")}
                </button>
                <button className="btn" disabled={!javaPath} onClick={() => testJava(javaPath)}>
                  {t("set.java_test")}
                </button>
              </div>
              {javaTest && (
                <p className={`row ${javaTest.ok ? "" : "error-text"}`} style={{ gap: 8, fontSize: 12.5 }}>
                  {javaTest.ok ? <CheckCircle2 size={15} color="var(--green)" /> : <XCircle size={15} />}
                  <span className="mono selectable">{javaTest.text}</span>
                </p>
              )}
            </>
          )}

          <h3 style={{ marginTop: 8 }}>{t("set.downloads")}</h3>
          <div className="field">
            <div className="row">
              <label className="label grow">{t("set.concurrency")}</label>
              <b className="mono">{conc}</b>
            </div>
            <Slider value={conc} min={2} max={64} onChange={setConc} />
            {conc !== s.concurrency && (
              <button className="btn sm primary" style={{ alignSelf: "flex-start" }} onClick={() => saveSettings({ concurrency: conc })}>
                {t("common.save")}
              </button>
            )}
          </div>

          <h3 style={{ marginTop: 8 }}>{t("set.data")}</h3>
          <div className="path-box mono selectable">{dataDir}</div>
          <div className="row">
            <button className="btn sm" onClick={() => api.openPath(dataDir).catch(fail)}>
              <FolderOpen size={14} />
              {t("set.open")}
            </button>
            <button
              className="btn sm"
              onClick={async () => {
                const p = await open({ directory: true, multiple: false });
                if (typeof p === "string") saveSettings({ dataDir: p });
              }}
            >
              {t("set.change")}
            </button>
            {s.dataDir && (
              <button className="btn sm ghost" onClick={() => saveSettings({ dataDir: null })}>
                <RotateCcw size={14} />
                {t("set.reset")}
              </button>
            )}
          </div>
          <p className="hint">{t("set.data_hint")}</p>
        </section>

        <section className="card pad set-card">
          <h3>{t("set.ms")}</h3>
          <p className="hint">{t("set.ms_hint")}</p>
          <div className="field">
            <label>{t("set.ms_client_id")}</label>
            <input
              className="input mono"
              style={{ fontSize: 12.5 }}
              value={clientId}
              placeholder="00000000-0000-0000-0000-000000000000"
              onChange={(e) => setClientId(e.target.value.trim())}
              onBlur={() => clientId !== s.msClientId && saveSettings({ msClientId: clientId })}
            />
          </div>
          <details className="howto">
            <summary>{t("set.ms_help")}</summary>
            <ol>
              <li>
                portal.azure.com → App registrations → New registration: «Personal Microsoft accounts only».
              </li>
              <li>Authentication → Add a platform → Mobile and desktop → redirect URI <code>https://login.microsoftonline.com/common/oauth2/nativeclient</code>; Allow public client flows — Yes.</li>
              <li>
                {t("set.ms_step_approve")}{" "}
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    api.openUrl("https://aka.ms/mce-reviewappid").catch(fail);
                  }}
                >
                  aka.ms/mce-reviewappid
                </a>
              </li>
              <li>{t("set.ms_step_paste")}</li>
            </ol>
          </details>

          <div className="about">
            <Logo size={46} />
            <div>
              <h2>ЛІЯ</h2>
              <p className="faint">{t("set.version", { v: version })}</p>
            </div>
          </div>
          <p className="hint">{t("set.about_text")}</p>
        </section>
      </div>
    </div>
  );
}
