import { invoke } from "@tauri-apps/api/core";

export type Loader = "vanilla" | "fabric" | "quilt" | "forge" | "neoforge";
export const LOADERS: Loader[] = ["vanilla", "fabric", "quilt", "forge", "neoforge"];

export interface ApiError {
  code: string;
  message: string;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  configDir: string;
  totalMemoryMb: number;
  os: string;
  arch: string;
}

export interface Settings {
  language: "uk" | "en";
  dataDir: string | null;
  memoryMb: number;
  javaPath: string | null;
  jvmArgs: string;
  width: number;
  height: number;
  fullscreen: boolean;
  onLaunch: "keep" | "minimize" | "close";
  concurrency: number;
  msClientId: string;
  selectedAccount: string | null;
  lastInstance: string | null;
  showFeatured: boolean;
  showSnapshots: boolean;
  uiScale: number;
}

export interface ModpackRef {
  projectId: string;
  versionId: string;
  name: string;
  versionName: string;
  iconUrl?: string | null;
}

export interface Instance {
  id: string;
  name: string;
  icon: string;
  mcVersion: string;
  loader: Loader;
  loaderVersion?: string | null;
  versionId?: string | null;
  memoryMb?: number | null;
  javaPath?: string | null;
  jvmArgs?: string | null;
  width?: number | null;
  height?: number | null;
  fullscreen?: boolean | null;
  created: number;
  lastPlayed?: number | null;
  playTime: number;
  featured?: string | null;
  modpack?: ModpackRef | null;
}

export interface NewInstance {
  name: string;
  mcVersion: string;
  loader: Loader;
  loaderVersion?: string | null;
  icon?: string;
}

export interface VersionEntry {
  id: string;
  kind: "release" | "snapshot" | "old_beta" | "old_alpha";
  releaseTime: string;
}

export interface LoaderVersion {
  id: string;
  name: string;
  stable: boolean;
}

export interface TaskEvent {
  id: number;
  title: string;
  tag: string | null;
  stage: string;
  unit: "bytes" | "items" | "";
  done: number;
  total: number;
  state: "running" | "done" | "error" | "cancelled";
  error: ApiError | null;
}

export interface LogLine {
  t: number;
  level: string;
  thread: string;
  text: string;
}

export interface CrashInfo {
  kind: string;
  detail?: string | null;
  reportPath?: string | null;
}

export interface GameEvent {
  instanceId: string;
  state: "running" | "exited";
  pid?: number;
  started?: number;
  code?: number | null;
  killed?: boolean;
  crash?: CrashInfo | null;
}

export interface Account {
  id: string;
  kind: "microsoft" | "offline";
  name: string;
  uuid: string;
  added: number;
}

export interface AccountsState {
  accounts: Account[];
  selected: string | null;
  msAvailable: boolean;
}

export interface SkinInfo {
  skin: string | null;
  /** Адреса текстури на серверах Mojang (лише Microsoft). */
  url?: string | null;
  slim: boolean;
  capes: { id: string; alias?: string | null; active: boolean; url?: string | null }[];
  source: "microsoft" | "mojang_name" | "default";
}

export interface LibSkin {
  id: string;
  name: string;
  slim: boolean;
  added: number;
  source: string;
  data: string;
}

export interface SkinsState {
  skins: LibSkin[];
  defaults: LibSkin[];
  applied: Record<string, { skin: string; url: string }>;
}

export interface ContentItem {
  file: string;
  name: string;
  enabled: boolean;
  isDir: boolean;
  size: number;
  modified: number;
  projectId?: string | null;
  version?: string | null;
  iconUrl?: string | null;
  description?: string | null;
}

export interface ContentUpdate {
  file: string;
  versionId: string;
  versionNumber: string;
  projectId: string;
}

export interface World {
  folder: string;
  icon?: string | null;
  modified: number;
  size: number;
}

export interface Shot {
  path: string;
  name: string;
  modified: number;
}

export interface Featured {
  id: string;
  name: string;
  address: string;
  port: number;
  mcVersion: string;
  loader: Loader;
  mods: string[];
  website?: string | null;
  descriptionUk: string;
  descriptionEn: string;
}

export interface ServerStatus {
  online: number;
  max: number;
  version: string;
  motd: unknown;
  favicon?: string | null;
  latencyMs: number;
  players: string[];
}

export interface NewsEntry {
  id: string;
  title: string;
  version: string;
  kind: string;
  image?: string | null;
  contentPath: string;
  date: string;
  shortText: string;
}

export interface McScan {
  path: string;
  exists: boolean;
  saves: { folder: string; size: number }[];
  resourcepacks: number;
  shaderpacks: number;
  mods: number;
  hasOptions: boolean;
  hasServers: boolean;
  lastVersion: [string, string] | null;
}

export type ContentKind = "mods" | "resourcepacks" | "shaderpacks";
export type ProjectType = "mod" | "modpack" | "resourcepack" | "shader";

export interface SearchParams {
  query: string;
  projectType: ProjectType;
  gameVersion?: string | null;
  loader?: string | null;
  categories?: string[];
  sort?: string;
  offset?: number;
  limit?: number;
}

export interface SearchHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  icon_url?: string | null;
  downloads: number;
  follows: number;
  date_modified: string;
  categories: string[];
  display_categories?: string[];
  project_type: string;
  gallery?: string[];
  featured_gallery?: string | null;
  color?: number | null;
}

export interface SearchResult {
  hits: SearchHit[];
  offset: number;
  limit: number;
  total_hits: number;
}

export interface LiaSession {
  api: string;
  token: string;
  name: string;
}

export interface LiaMe {
  name: string;
  email: string | null;
  emailVerified: boolean;
  mailEnabled: boolean;
  created: number;
  profiles: { name: string; kind: "offline" | "mojang" }[];
}

export interface LiaDevice {
  id: string;
  device: string;
  created: number;
  lastUsed: number;
  current: boolean;
}

export interface LiaPresence {
  state: "online" | "playing";
  since: number;
  game?: { name?: string | null; mcVersion?: string | null; loader?: string | null; pack?: string | null };
  world?: { code: string; address: string; licensedOnly?: boolean };
}

export interface LiaUser {
  name: string;
  uuid: string;
  skin: string | null;
  slim: boolean;
  bio: string;
  presence?: LiaPresence | null;
}

export interface LiaFriends {
  friends: LiaUser[];
  incoming: LiaUser[];
  outgoing: LiaUser[];
}

export interface LiaPack {
  id: string;
  owner: string;
  mine: boolean;
  name: string;
  description: string;
  mcVersion: string;
  loader: Loader;
  visibility: "friends" | "public";
  version: number;
  size: number;
  updated: number;
  subscribed: boolean;
  subscribers: number;
}

export interface PackSelection {
  include: string[];
  exclude: string[];
}

/** Що є в теці збірки і звідки кожен файл отримає гравець. */
export interface PublishPlan {
  base: { projectId: string; versionId: string; name: string; versionName: string } | null;
  files: { path: string; size: number; kind: "base" | "link" | "upload" | "stored" | "big" }[];
  selection: PackSelection;
  packId: string | null;
  shareOptions: boolean;
  options: { lang?: string; resourcePacks: string[] } | null;
}

export interface LiaEvent {
  id: number;
  user: string;
  kind: "friend" | "pack_published" | "pack_updated";
  pack: string | null;
  data: { name?: string; version?: number; a?: string; b?: string };
  created: number;
}

/* eslint-disable @typescript-eslint/no-explicit-any */
export type Json = any;

export function errorOf(e: unknown): ApiError {
  if (e && typeof e === "object" && "code" in e) return e as ApiError;
  return { code: "unknown", message: String(e) };
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  launcherOutdated: () => invoke<boolean>("launcher_outdated"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  openPath: (path: string, reveal = false) => invoke<void>("open_path", { path, reveal }),
  taskCancel: (id: number) => invoke<void>("task_cancel", { id }),

  settingsGet: () => invoke<Settings>("settings_get"),
  settingsSet: (settings: Settings) => invoke<Settings>("settings_set", { settings }),
  javaProbe: (path: string) => invoke<{ line: string; major: number | null }>("java_probe", { path }),

  versions: (refresh = false) =>
    invoke<{ latest: { release: string; snapshot: string }; versions: VersionEntry[] }>("versions_list", { refresh }),
  loaderVersions: (loader: Loader, mc: string) => invoke<LoaderVersion[]>("loader_versions", { loader, mc }),
  loaderGames: (loader: Loader) => invoke<string[]>("loader_games", { loader }),

  instances: () => invoke<Instance[]>("instances_list"),
  instance: (id: string) => invoke<Instance>("instance_get", { id }),
  instanceCreate: (spec: NewInstance) => invoke<Instance>("instance_create", { spec }),
  instanceUpdate: (id: string, patch: Record<string, unknown>) => invoke<Instance>("instance_update", { id, patch }),
  instanceSetIconFile: (id: string, path: string) => invoke<Instance>("instance_set_icon_file", { id, path }),
  instanceDelete: (id: string) => invoke<void>("instance_delete", { id }),
  instanceDuplicate: (id: string, suffix: string) => invoke<Instance>("instance_duplicate", { id, suffix }),
  instanceFolder: (id: string, sub?: string) => invoke<string>("instance_folder", { id, sub }),
  launch: (id: string, server?: { host: string; port?: number }, world?: string, friends?: boolean) => invoke<void>("instance_launch", { id, server, world, friends }),
  repair: (id: string) => invoke<void>("instance_repair", { id }),
  kill: (id: string) => invoke<boolean>("instance_kill", { id }),
  gameLog: (id: string) => invoke<LogLine[]>("game_log", { id }),
  running: () => invoke<{ instanceId: string; pid: number; started: number }[]>("games_running"),

  featured: () => invoke<Featured>("featured_get"),
  featuredPlay: () => invoke<string>("featured_play"),
  ping: (host: string, port?: number) => invoke<ServerStatus>("server_ping", { host, port }),

  content: (id: string, kind: ContentKind) => invoke<ContentItem[]>("content_list", { id, kind }),
  contentToggle: (id: string, kind: ContentKind, file: string) => invoke<string>("content_toggle", { id, kind, file }),
  contentRemove: (id: string, kind: ContentKind, file: string) => invoke<void>("content_remove", { id, kind, file }),
  contentAdd: (id: string, kind: ContentKind, files: string[]) => invoke<number>("content_add", { id, kind, files }),
  contentIdentify: (id: string, kind: ContentKind) => invoke<ContentItem[]>("content_identify", { id, kind }),
  contentUpdates: (id: string, kind: ContentKind) => invoke<ContentUpdate[]>("content_updates", { id, kind }),
  contentUpdate: (id: string, kind: ContentKind, u: ContentUpdate) =>
    invoke<void>("content_update", { id, kind, file: u.file, projectId: u.projectId, versionId: u.versionId }),
  worlds: (id: string) => invoke<World[]>("worlds_list", { id }),
  screenshots: (id: string) => invoke<Shot[]>("screenshots_list", { id }),
  trashGameFile: (id: string, rel: string) => invoke<void>("game_file_trash", { id, rel }),

  search: (params: SearchParams) => invoke<SearchResult>("mr_search", { params }),
  project: (id: string) => invoke<Json>("mr_project", { id }),
  projectVersions: (id: string, loaders: string[], gameVersions: string[]) =>
    invoke<Json[]>("mr_versions", { id, loaders, gameVersions }),
  install: (instanceId: string, projectId: string, versionId?: string) =>
    invoke<{ projectId: string; title: string; file: string }[]>("mr_install", { instanceId, projectId, versionId }),
  installModpack: (versionId: string, title: string) => invoke<Instance>("mr_install_modpack", { versionId, title }),
  importMrpack: (path: string) => invoke<Instance>("mrpack_import", { path }),

  accounts: () => invoke<AccountsState>("accounts_list"),
  addOffline: (name: string) => invoke<Account>("account_add_offline", { name }),
  selectAccount: (id: string) => invoke<void>("account_select", { id }),
  removeAccount: (id: string) => invoke<void>("account_remove", { id }),
  loginMicrosoft: () => invoke<Account>("account_login_ms"),
  skin: (id: string) => invoke<SkinInfo>("account_skin", { id }),
  setCape: (id: string, capeId: string | null) => invoke<void>("cape_set", { id, capeId }),
  skins: () => invoke<SkinsState>("skins_list"),
  skinAddFile: (path: string) => invoke<LibSkin>("skin_add_file", { path }),
  skinAddNick: (nick: string) => invoke<LibSkin>("skin_add_nick", { nick }),
  skinSaveCurrent: (id: string) => invoke<LibSkin>("skin_save_current", { id }),
  skinUpdate: (id: string, patch: { name?: string; slim?: boolean }) => invoke<void>("skin_update", { id, ...patch }),
  skinRemove: (id: string) => invoke<void>("skin_remove", { id }),
  skinSavePng: (data: string, name: string, slim: boolean, replace?: string | null) => invoke<LibSkin>("skin_save_png", { data, name, slim, replace }),
  skinExport: (path: string, data: string) => invoke<void>("skin_export", { path, data }),
  skinApply: (accountId: string, skinId: string, slim: boolean) => invoke<void>("skin_apply", { accountId, skinId, slim }),

  mcdirScan: (path?: string) => invoke<McScan>("mcdir_scan", { path }),
  mcdirImport: (path: string, spec: NewInstance, what: { saves: string[]; resourcepacks: boolean; shaderpacks: boolean; mods: boolean; options: boolean }) =>
    invoke<Instance>("mcdir_import", { path, spec, what }),

  news: () => invoke<NewsEntry[]>("news_list"),

  liaLogin: (name: string, password: string, email?: string) => invoke<{ name: string }>("lia_login", { name, password, email }),
  liaPublic: (path: "/api/password/forgot" | "/api/password/reset", body: unknown) => invoke<void>("lia_public", { path, body }),
  liaLogout: () => invoke<void>("lia_logout"),
  liaSession: () => invoke<LiaSession | null>("lia_session"),
  lia: <T = Json>(method: "GET" | "POST" | "PUT" | "DELETE", path: string, body?: unknown) => invoke<T>("lia_request", { method, path, body }),
  liaPublishPlan: (instanceId: string) => invoke<PublishPlan>("lia_publish_plan", { instanceId }),
  liaPublish: (instanceId: string, visibility: "friends" | "public", description: string, packId: string | null, selection: PackSelection, shareOptions: boolean) =>
    invoke<LiaPack>("lia_publish", { instanceId, visibility, description, packId, selection, shareOptions }),
  liaInstallPack: (packId: string) => invoke<Instance>("lia_install_pack", { packId }),
  liaUpdatePack: (instanceId: string) => invoke<Instance>("lia_update_pack", { instanceId }),
  liaShareWorld: (instanceId: string, port: number) => invoke<{ code: string; address: string; licensedOnly: boolean }>("lia_share_world", { instanceId, port }),
  liaUnshare: (instanceId: string) => invoke<void>("lia_unshare", { instanceId }),
};
