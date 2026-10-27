// Соцмережа ЛІЇ в інтерфейсі: сесія ЛІЯ ID, статуси друзів через WebSocket,
// автоматичне відкриття LAN-світу друзям через ретранслятор.
import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type GameEvent, type LiaFriends, type LiaPresence, type LiaSession, type LogLine } from "./api";
import { t } from "./i18n";
import { fail, getState, refreshInstances, toast } from "./store";

interface LiaState {
  session: LiaSession | null;
  online: boolean;
  friends: LiaFriends | null;
  /** Відкриті друзям світи: збірка → адреса. */
  shares: Record<string, { code: string; address: string; licensedOnly: boolean }>;
  /** Лічильник змін стрічки й збірок — сторінки перезавантажують дані, коли він росте. */
  rev: number;
}

let st: LiaState = { session: null, online: false, friends: null, shares: {}, rev: 0 };
const subs = new Set<() => void>();
const set = (p: Partial<LiaState>) => {
  st = { ...st, ...p };
  subs.forEach((f) => f());
};

export function useLia<T>(sel: (s: LiaState) => T): T {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => sel(st),
  );
}

export const liaState = () => st;

let ws: WebSocket | null = null;
let retry: ReturnType<typeof setTimeout> | null = null;

export async function loadFriends() {
  if (!st.session) return;
  try {
    set({ friends: await api.lia<LiaFriends>("GET", "/api/friends") });
  } catch (e) {
    fail(e);
  }
}

/** Що зараз робить гравець — те й бачать друзі. */
function presence() {
  const s = getState();
  const id = Object.keys(s.running)[0];
  const inst = id ? s.instances.find((i) => i.id === id) : undefined;
  if (!inst) return { type: "presence", state: "online" };
  const pack = inst.modpack?.projectId.startsWith("lia:") ? inst.modpack.projectId.slice(4) : null;
  return {
    type: "presence",
    state: "playing",
    game: { name: inst.name, mcVersion: inst.mcVersion, loader: inst.loader, pack },
    world: st.shares[inst.id] ? { code: st.shares[inst.id].code, licensedOnly: st.shares[inst.id].licensedOnly } : null,
  };
}

export function sendPresence() {
  if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(presence()));
}

function connect() {
  if (retry) clearTimeout(retry);
  ws?.close();
  ws = null;
  const s = st.session;
  if (!s) return set({ online: false });
  const sock = new WebSocket(`${s.api.replace(/^http/, "ws")}/api/ws?token=${encodeURIComponent(s.token)}`);
  ws = sock;
  sock.onopen = () => {
    set({ online: true });
    sendPresence();
  };
  sock.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.type === "presence") {
      const f = st.friends;
      if (f) set({ friends: { ...f, friends: f.friends.map((x) => (x.name === m.name ? { ...x, presence: m.presence as LiaPresence | null } : x)) } });
    } else if (m.type === "friends") {
      if (m.request) toast("info", t("lia.request_toast", { name: m.request }));
      loadFriends();
    } else if (m.type === "feed") {
      set({ rev: st.rev + 1 });
    } else if (m.type === "pack") {
      const inst = getState().instances.find((i) => i.modpack?.projectId === `lia:${m.id}`);
      if (inst) toast("info", t("lia.pack_update_toast", { name: inst.name }));
      set({ rev: st.rev + 1 });
    }
  };
  sock.onclose = () => {
    if (ws !== sock) return;
    set({ online: false });
    // Сервер перезапустився або зник інтернет — пробуємо знову.
    retry = setTimeout(connect, 8000);
  };
}

export async function refreshLia() {
  try {
    const session = await api.liaSession();
    const changed = session?.token !== st.session?.token;
    set({ session, ...(session ? {} : { friends: null }) });
    if (changed) {
      connect();
      loadFriends();
    }
  } catch {
    // Немає мережі або токен відкликано — соцмережа просто недоступна.
    set({ session: null, friends: null });
    connect();
  }
}

// LAN-світ: «Started serving on 54321» (1.13+) або «Started on 54321» (старі версії).
const LAN = /Started (?:serving )?on (\d{2,5})\b/;

async function shareWorld(instanceId: string, port: number) {
  if (!st.session) return;
  try {
    const r = await api.liaShareWorld(instanceId, port);
    set({ shares: { ...st.shares, [instanceId]: r } });
    sendPresence();
    toast("success", t("lia.world_shared"), r.licensedOnly ? t("lia.world_shared_licensed") : t("lia.world_shared_text"));
  } catch (e) {
    fail(e);
  }
}

function unshare(instanceId: string) {
  if (!st.shares[instanceId]) return;
  const shares = { ...st.shares };
  delete shares[instanceId];
  set({ shares });
  api.liaUnshare(instanceId).catch(() => {});
  sendPresence();
}

export async function initLia() {
  await listen<{ instanceId: string; lines: LogLine[] }>("game-log", ({ payload }) => {
    for (const l of payload.lines) {
      const m = LAN.exec(l.text);
      if (m) shareWorld(payload.instanceId, Number(m[1]));
      else if (l.text.includes("Stopping server")) unshare(payload.instanceId);
    }
  });
  await listen<GameEvent>("game", ({ payload }) => {
    if (payload.state === "exited") unshare(payload.instanceId);
    // Стан запущених збірок оновлюється в основному сховищі тим самим подієм — чекаємо його.
    setTimeout(sendPresence, 50);
  });
  await listen<{ instanceId: string }>("lia-share-ended", ({ payload }) => unshare(payload.instanceId));
  await listen("lia-session", () => refreshLia());
  await listen<{ message: string }>("lia-warning", ({ payload }) => toast("info", t("lia.warning"), payload.message));
  await refreshLia();
}

/** Встановити збірку друга (нова збірка в списку). */
export async function installPack(id: string) {
  const inst = await api.liaInstallPack(id);
  await refreshInstances();
  return inst;
}
