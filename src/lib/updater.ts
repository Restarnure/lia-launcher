import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "./api";
import { t } from "./i18n";
import { getState, setState, toast } from "./store";

export type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "none" }
  | { status: "available"; update: Update }
  | { status: "downloading"; update: Update; done: number; total: number }
  | { status: "installing"; update: Update }
  | { status: "error"; message: string };

let announced = "";

/** Перевірка оновлення. Автоматична — мовчки при помилці, ручна — з повідомленням. */
export async function checkForUpdate(manual = false) {
  const cur = getState().update.status;
  if (cur === "checking" || cur === "downloading" || cur === "installing") return;
  setState({ update: { status: "checking" } });
  try {
    const update = await check({ timeout: 20000 });
    if (update) {
      setState({ update: { status: "available", update } });
      if (announced !== update.version) {
        announced = update.version;
        toast("info", t("upd.toast", { v: update.version }), t("upd.toast_text"));
      }
    } else {
      setState({ update: { status: "none" } });
      if (manual) toast("success", t("upd.none"));
    }
  } catch (e) {
    setState({ update: manual ? { status: "error", message: String(e) } : { status: "idle" } });
    const msg = String(e);
    if (manual) toast("error", t("upd.error"), /valid release JSON|404|Not Found/i.test(msg) ? t("upd.unreachable") : msg);
  }
}

/** Завантажити, встановити (тихий режим NSIS) і перезапустити лаунчер. */
export async function installUpdate() {
  const st = getState().update;
  if (st.status !== "available") return;
  const update = st.update;
  let done = 0;
  let total = 0;
  try {
    await update.downloadAndInstall((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? 0;
      if (ev.event === "Progress") done += ev.data.chunkLength;
      if (ev.event === "Finished") setState({ update: { status: "installing", update } });
      else setState({ update: { status: "downloading", update, done, total } });
    });
    setState({ update: { status: "installing", update } });
    await relaunch();
  } catch (e) {
    setState({ update: { status: "available", update } });
    const msg = String(e);
    toast("error", t("upd.failed"), /signature/i.test(msg) ? t("upd.bad_signature") : msg);
  }
}

/** Перевірка одразу після старту, далі раз на 6 годин. Застарілу версію сервер ЛІЇ не пускає, і тоді оновлення обов'язкове. */
export function scheduleUpdateChecks() {
  if (import.meta.env.DEV) return;
  const run = () => {
    checkForUpdate(false);
    api.launcherOutdated().then((old) => old && setState({ mustUpdate: true })).catch(() => {});
  };
  run();
  setInterval(run, 6 * 60 * 60 * 1000);
}
