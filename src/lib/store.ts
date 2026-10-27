import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  api,
  errorOf,
  type AccountsState,
  type AppInfo,
  type CrashInfo,
  type GameEvent,
  type Instance,
  type LogLine,
  type ProjectType,
  type Settings,
  type TaskEvent,
} from "./api";
import { errText, setLang, t } from "./i18n";
import type { UpdateState } from "./updater";

export type InstanceTab = "mods" | "resourcepacks" | "shaderpacks" | "worlds" | "screenshots" | "console" | "settings";

export type Route =
  | { page: "home" }
  | { page: "instances" }
  | { page: "instance"; id: string; tab?: InstanceTab }
  | { page: "browse"; type?: ProjectType; instanceId?: string | null }
  | { page: "accounts" }
  | { page: "skins"; accountId?: string }
  | { page: "skin-editor"; start?: { name: string; data: string | null; slim: boolean; libId?: string | null } }
  | { page: "friends" }
  | { page: "settings" };

export interface Toast {
  id: number;
  kind: "error" | "success" | "info";
  title: string;
  text?: string;
}

interface State {
  ready: boolean;
  info: AppInfo | null;
  settings: Settings | null;
  instances: Instance[];
  accounts: AccountsState;
  tasks: Record<number, TaskEvent>;
  running: Record<string, { pid: number; started: number }>;
  logs: Record<string, LogLine[]>;
  crash: { instanceId: string; info: CrashInfo; code: number | null } | null;
  route: Route;
  history: Route[];
  toasts: Toast[];
  newInstanceOpen: boolean;
  addAccountOpen: boolean;
  update: UpdateState;
  updateOpen: boolean;
  /** Що зараз встановлюється з каталогу: «<збірка або modpack>:<проєкт>». Живе довше за сторінку. */
  installing: Record<string, true>;
}

let state: State = {
  ready: false,
  info: null,
  settings: null,
  instances: [],
  accounts: { accounts: [], selected: null, msAvailable: false },
  tasks: {},
  running: {},
  logs: {},
  crash: null,
  route: { page: "home" },
  history: [],
  toasts: [],
  newInstanceOpen: false,
  addAccountOpen: false,
  update: { status: "idle" },
  updateOpen: false,
  installing: {},
};

const listeners = new Set<() => void>();

export function getState(): State {
  return state;
}

export function setState(patch: Partial<State> | ((s: State) => Partial<State>)) {
  state = { ...state, ...(typeof patch === "function" ? patch(state) : patch) };
  listeners.forEach((l) => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** Вибір частини стану. Селектор має повертати наявне значення (не новий об'єкт). */
export function useStore<T>(sel: (s: State) => T): T {
  return useSyncExternalStore(subscribe, () => sel(state));
}

// ------------------------------------------------------------ навігація

export function navigate(route: Route) {
  setState((s) => ({ route, history: [...s.history.slice(-30), s.route] }));
}

export function back() {
  setState((s) => {
    const prev = s.history[s.history.length - 1];
    return prev ? { route: prev, history: s.history.slice(0, -1) } : {};
  });
}

// ------------------------------------------------------------ сповіщення

let toastId = 1;

export function toast(kind: Toast["kind"], title: string, text?: string) {
  // Те саме повідомлення, що вже на екрані, не дублюємо.
  if (state.toasts.some((x) => x.kind === kind && x.title === title && x.text === text)) return;
  const id = toastId++;
  setState((s) => ({ toasts: [...s.toasts.slice(-3), { id, kind, title, text }] }));
  setTimeout(() => dismissToast(id), kind === "error" ? 9000 : 4500);
}

export function dismissToast(id: number) {
  setState((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
}

// Коди, для яких технічна деталь допомагає (який файл, яка відповідь сервера).
const DETAILED = new Set(["net", "http", "io", "json", "zip", "integrity", "loader_install", "loader_broken", "java_broken", "java_unavailable", "ms_oauth", "ms_xbox", "ms_mc", "ms_app_not_approved", "skin_upload", "invalid_modpack", "unknown"]);

/** Показує помилку з ядра людською мовою (скасування — мовчки). */
export function fail(e: unknown, title?: string) {
  const err = errorOf(e);
  if (err.code === "cancelled") return;
  if (err.code === "no_account") {
    setState({ addAccountOpen: true });
    return;
  }
  const text = errText(err);
  const detail = DETAILED.has(err.code) && err.message && text !== err.message ? err.message : undefined;
  toast("error", title ?? text, title ? text : detail);
}

// ------------------------------------------------------------ дані

export async function refreshInstances() {
  try {
    setState({ instances: await api.instances() });
  } catch (e) {
    fail(e);
  }
}

export async function refreshAccounts() {
  try {
    setState({ accounts: await api.accounts() });
  } catch (e) {
    fail(e);
  }
}

const applyScale = (scale: number) => getCurrentWebview().setZoom(scale).catch(fail);

export async function saveSettings(patch: Partial<Settings>) {
  const cur = state.settings;
  if (!cur) return;
  try {
    const s = await api.settingsSet({ ...cur, ...patch });
    if (s.language !== cur.language) setLang(s.language);
    if (s.uiScale !== cur.uiScale) applyScale(s.uiScale);
    setState({ settings: s, info: state.info ? { ...state.info } : null });
    if (patch.msClientId !== undefined) refreshAccounts();
    if (patch.dataDir !== undefined) {
      setState({ info: await api.appInfo() });
      refreshInstances();
    }
    return s;
  } catch (e) {
    fail(e);
  }
}

export async function launch(id: string, server?: { host: string; port?: number }, world?: string, friends?: boolean) {
  try {
    await api.launch(id, server, world, friends);
  } catch (e) {
    fail(e);
  }
}

export async function kill(id: string) {
  await api.kill(id).catch(fail);
}

/** Задача, що зараз готує конкретну збірку (для прогресу на кнопці «Грати»). */
export function useInstanceTask(id: string | undefined): TaskEvent | undefined {
  return useStore((s) => (id ? Object.values(s.tasks).find((t) => t.tag === id && t.state === "running") : undefined));
}

// ------------------------------------------------------------ запуск застосунку

const LOG_KEEP = 8000;

export async function init() {
  const [settings, info] = await Promise.all([api.settingsGet(), api.appInfo()]);
  setLang(settings.language);
  applyScale(settings.uiScale);
  setState({ settings, info });
  await Promise.all([refreshInstances(), refreshAccounts()]);
  const running = await api.running().catch(() => []);
  setState({
    ready: true,
    running: Object.fromEntries(running.map((r) => [r.instanceId, { pid: r.pid, started: r.started }])),
  });

  await listen<TaskEvent>("task", ({ payload }) => {
    setState((s) => ({ tasks: { ...s.tasks, [payload.id]: payload } }));
    if (payload.state !== "running") {
      setTimeout(() => {
        setState((s) => {
          const tasks = { ...s.tasks };
          delete tasks[payload.id];
          return { tasks };
        });
      }, 8000);
    }
  });

  await listen<GameEvent>("game", ({ payload: g }) => {
    if (g.state === "running") {
      setState((s) => ({
        running: { ...s.running, [g.instanceId]: { pid: g.pid ?? 0, started: g.started ?? Date.now() } },
        logs: { ...s.logs, [g.instanceId]: [] },
        // Ядро вже запам'ятало цю збірку як останню — головна має показувати її.
        settings: s.settings ? { ...s.settings, lastInstance: g.instanceId } : s.settings,
      }));
      const inst = state.instances.find((i) => i.id === g.instanceId);
      toast("success", t("game.started", { name: inst?.name ?? "" }));
    } else {
      setState((s) => {
        const running = { ...s.running };
        delete running[g.instanceId];
        return { running, crash: g.crash ? { instanceId: g.instanceId, info: g.crash, code: g.code ?? null } : s.crash };
      });
    }
  });

  await listen<{ instanceId: string; lines: LogLine[] }>("game-log", ({ payload }) => {
    setState((s) => {
      const prev = s.logs[payload.instanceId] ?? [];
      const next = prev.length + payload.lines.length > LOG_KEEP ? [...prev, ...payload.lines].slice(-LOG_KEEP) : [...prev, ...payload.lines];
      return { logs: { ...s.logs, [payload.instanceId]: next } };
    });
  });

  await listen("instances-changed", () => refreshInstances());
  await listen("accounts-changed", async () => {
    refreshAccounts();
    // Обраний акаунт зберігає ядро — тримаємо копію налаштувань свіжою, щоб не перезаписати його.
    const fresh = await api.settingsGet().catch(() => null);
    if (fresh) setState({ settings: fresh });
  });
}
