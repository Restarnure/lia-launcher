// Випуск нової версії ЛІЇ.
//   node scripts/release.mjs 1.0.1 "Що нового (markdown або шлях до .md)" [--publish]
// Піднімає версію, збирає підписаний інсталятор, кладе в release/<версія>/ інсталятор
// і latest.json. З --publish створює реліз на GitHub — лаунчери гравців побачать оновлення.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO = "Restarnure/lia-launcher";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [version, notesArg = "", ...flags] = process.argv.slice(2);

function die(msg) {
  console.error(`✖ ${msg}`);
  process.exit(1);
}
const file = (p) => path.join(root, p);

if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) die("вкажіть версію: node scripts/release.mjs 1.0.1 \"що нового\" [--publish]");
const notes = notesArg && fs.existsSync(notesArg) ? fs.readFileSync(notesArg, "utf8") : notesArg;
const keyPath = process.env.LIA_UPDATER_KEY ?? path.join(os.homedir(), ".tauri", "lia-updater.key");
if (!fs.existsSync(keyPath)) die(`немає ключа підпису ${keyPath} — без нього лаунчери не приймуть оновлення`);

// 1. Версія в усіх трьох місцях.
const conf = JSON.parse(fs.readFileSync(file("src-tauri/tauri.conf.json"), "utf8"));
conf.version = version;
fs.writeFileSync(file("src-tauri/tauri.conf.json"), JSON.stringify(conf, null, 2) + "\n");
const pkg = JSON.parse(fs.readFileSync(file("package.json"), "utf8"));
pkg.version = version;
fs.writeFileSync(file("package.json"), JSON.stringify(pkg, null, 2) + "\n");
const cargo = fs.readFileSync(file("src-tauri/Cargo.toml"), "utf8");
fs.writeFileSync(file("src-tauri/Cargo.toml"), cargo.replace(/^version = "[^"]*"/m, `version = "${version}"`));

// 2. Підписана збірка.
console.log(`▶ збірка ЛІЯ ${version}…`);
const build = spawnSync("npm run tauri build", {
  cwd: root,
  stdio: "inherit",
  shell: true,
  env: {
    ...process.env,
    TAURI_SIGNING_PRIVATE_KEY: fs.readFileSync(keyPath, "utf8").trim(),
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  },
});
if (build.status !== 0) die("збірка не вдалась");

// 3. Інсталятор з латинською назвою (для посилань) і latest.json.
const nsis = file("src-tauri/target/release/bundle/nsis");
const exe = fs.readdirSync(nsis).find((f) => f.endsWith(`_${version}_x64-setup.exe`));
if (!exe || !fs.existsSync(path.join(nsis, `${exe}.sig`))) die("не знайдено інсталятор або підпис .sig");
const out = file(`release/${version}`);
fs.mkdirSync(out, { recursive: true });
const asset = `LIA_${version}_x64-setup.exe`;
fs.copyFileSync(path.join(nsis, exe), path.join(out, asset));
const latest = {
  version,
  notes: notes || "Виправлення й покращення.",
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: fs.readFileSync(path.join(nsis, `${exe}.sig`), "utf8").trim(),
      url: `https://github.com/${REPO}/releases/download/v${version}/${asset}`,
    },
  },
};
fs.writeFileSync(path.join(out, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
console.log(`✔ готово: ${out}`);

// 4. Публікація.
if (flags.includes("--publish")) {
  const gh = spawnSync(
    "gh",
    ["release", "create", `v${version}`, path.join(out, asset), path.join(out, "latest.json"), "--repo", REPO, "--title", `ЛІЯ ${version}`, "--notes", latest.notes],
    { cwd: root, stdio: "inherit" },
  );
  if (gh.status !== 0) die("не вдалося створити реліз на GitHub");
  console.log(`✔ опубліковано: https://github.com/${REPO}/releases/tag/v${version}`);
} else {
  console.log("  Щоб гравці отримали оновлення, запустіть те саме з --publish.");
}
