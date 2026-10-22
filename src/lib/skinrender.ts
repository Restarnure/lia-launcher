import { SkinViewer } from "skinview3d";

// 3D-мініатюри скінів для бібліотеки: один невидимий рендерер на всі картки, черга, кеш.

let viewer: SkinViewer | null = null;
let queue: Promise<unknown> = Promise.resolve();
const cache = new Map<string, string>();

function ensure(): SkinViewer {
  if (viewer) return viewer;
  const canvas = document.createElement("canvas");
  viewer = new SkinViewer({ canvas, width: 200, height: 260, preserveDrawingBuffer: true, renderPaused: true, fov: 38, zoom: 0.86 });
  viewer.controls.enabled = false;
  // Три чверті, як у каталогах скінів.
  viewer.playerObject.rotation.y = 0.55;
  viewer.playerObject.skin.leftArm.rotation.x = 0.25;
  viewer.playerObject.skin.rightArm.rotation.x = -0.25;
  viewer.playerObject.skin.leftLeg.rotation.x = -0.2;
  viewer.playerObject.skin.rightLeg.rotation.x = 0.2;
  return viewer;
}

/** PNG-знімок 3D-моделі скіна. Ключ — щоб не рендерити той самий скін двічі. */
export function skinThumb(key: string, data: string, slim: boolean): Promise<string> {
  const id = `${key}:${slim ? "s" : "c"}`;
  const hit = cache.get(id);
  if (hit) return Promise.resolve(hit);
  const job = queue.then(async () => {
    if (cache.has(id)) return cache.get(id)!;
    const v = ensure();
    await v.loadSkin(data, { model: slim ? "slim" : "default" });
    v.render();
    const url = v.canvas.toDataURL("image/png");
    cache.set(id, url);
    return url;
  });
  queue = job.catch(() => undefined);
  return job;
}
