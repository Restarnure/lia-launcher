import { useCallback, useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { SkinViewer } from "skinview3d";
import { Raycaster, Vector2, type Intersection, type Object3D } from "three";
import {
  ArrowLeft,
  Brush,
  Download,
  Eraser,
  FlipHorizontal2,
  FolderOpen,
  Grid3x3,
  Moon,
  PaintBucket,
  Pipette,
  Redo2,
  RotateCcw,
  Save,
  Sparkles,
  Sun,
  Undo2,
} from "lucide-react";
import { api, type LibSkin } from "../lib/api";
import { t, type Key } from "../lib/i18n";
import { mirror, PARTS, rectAt, rects, type Layer, type Part, type Rect } from "../lib/skinmap";
import { back, fail, toast } from "../lib/store";
import { confirmDialog, Modal, Seg, Switch } from "../components/ui";

type Tool = "pencil" | "eraser" | "fill" | "picker" | "darken" | "lighten" | "noise";
type RGBA = [number, number, number, number];

const TOOLS: { id: Tool; key: string; icon: React.ReactNode; label: Key }[] = [
  { id: "pencil", key: "b", icon: <Brush size={18} />, label: "editor.pencil" },
  { id: "eraser", key: "e", icon: <Eraser size={18} />, label: "editor.eraser" },
  { id: "fill", key: "g", icon: <PaintBucket size={18} />, label: "editor.fill" },
  { id: "picker", key: "i", icon: <Pipette size={18} />, label: "editor.picker" },
  { id: "darken", key: "d", icon: <Moon size={18} />, label: "editor.darken" },
  { id: "lighten", key: "l", icon: <Sun size={18} />, label: "editor.lighten" },
  { id: "noise", key: "n", icon: <Sparkles size={18} />, label: "editor.noise" },
];

const PALETTE = [
  "#000000", "#3f3f3f", "#7f7f7f", "#bfbfbf", "#ffffff", "#5b3b23", "#8a5a3b", "#c58c62", "#e8b896", "#f6d7b0",
  "#7a1f1f", "#c0392b", "#e67e22", "#f1c40f", "#27ae60", "#1e824c", "#2980b9", "#1f3a93", "#8e44ad", "#d35d9e",
];

const PART_LABEL: Record<Part, Key> = {
  head: "editor.part.head",
  body: "editor.part.body",
  rightArm: "editor.part.right_arm",
  leftArm: "editor.part.left_arm",
  rightLeg: "editor.part.right_leg",
  leftLeg: "editor.part.left_leg",
};

const hex = (c: RGBA) => "#" + c.slice(0, 3).map((v) => v.toString(16).padStart(2, "0")).join("");
const parseHex = (h: string): RGBA | null => {
  const m = /^#?([0-9a-f]{6})$/i.exec(h.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 255];
};

export interface EditorStart {
  name: string;
  data: string | null;
  slim: boolean;
  libId?: string | null;
}

export function SkinEditor({ start }: { start?: EditorStart }) {
  const tex = useRef<HTMLCanvasElement>(document.createElement("canvas"));
  const [rev, setRev] = useState(0);
  const bump = useCallback(() => setRev((r) => r + 1), []);
  const undo = useRef<ImageData[]>([]);
  const redo = useRef<ImageData[]>([]);
  const [name, setName] = useState(start?.name ?? t("editor.new_name"));
  const [libId, setLibId] = useState<string | null>(start?.libId ?? null);
  const [slim, setSlim] = useState(start?.slim ?? false);
  const [tool, setTool] = useState<Tool>("pencil");
  const [color, setColor] = useState<RGBA>([197, 140, 98, 255]);
  const [recent, setRecent] = useState<string[]>([]);
  const [size, setSize] = useState(1);
  const [mirrorOn, setMirrorOn] = useState(false);
  const [layer, setLayer] = useState<Layer>("inner");
  const [showOuter, setShowOuter] = useState(true);
  const [hidden, setHidden] = useState<Set<Part>>(new Set());
  const [pose, setPose] = useState(true);
  const [grid, setGrid] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [openPicker, setOpenPicker] = useState(false);

  // Стан, потрібний обробникам миші без перестворення.
  const st = useRef({ tool, color, size, mirrorOn, slim, layer });
  st.current = { tool, color, size, mirrorOn, slim, layer };

  const ctx = () => tex.current.getContext("2d", { willReadFrequently: true })!;

  const load = useCallback(
    async (data: string | null) => {
      const c = tex.current;
      c.width = 64;
      c.height = 64;
      const g = ctx();
      g.clearRect(0, 0, 64, 64);
      if (data) {
        // Старі скіни 64×32 нормалізує skinview3d — беремо вже готову текстуру з нього.
        const img = new Image();
        img.src = data;
        await img.decode();
        const tmp = new SkinViewer({ width: 1, height: 1, renderPaused: true });
        tmp.loadSkin(img, { model: "default" });
        g.drawImage(tmp.skinCanvas, 0, 0, 64, 64);
        tmp.dispose();
      }
      undo.current = [];
      redo.current = [];
      setDirty(false);
      bump();
    },
    [bump],
  );

  useEffect(() => {
    load(start?.data ?? null).catch(fail);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------- малювання
  const snapshot = () => {
    undo.current.push(ctx().getImageData(0, 0, 64, 64));
    if (undo.current.length > 100) undo.current.shift();
    redo.current = [];
    setDirty(true);
  };
  const history = (from: React.MutableRefObject<ImageData[]>, to: React.MutableRefObject<ImageData[]>) => {
    const img = from.current.pop();
    if (!img) return;
    to.current.push(ctx().getImageData(0, 0, 64, 64));
    ctx().putImageData(img, 0, 0);
    setDirty(true);
    bump();
  };

  const stroke = useRef<{ visited: Set<number>; last: [number, number] | null } | null>(null);

  const pick = (x: number, y: number) => {
    const d = ctx().getImageData(x, y, 1, 1).data;
    if (d[3] === 0) return;
    setColor([d[0], d[1], d[2], 255]);
    setTool((cur) => (cur === "picker" ? "pencil" : cur));
  };

  /** Пікселі пензля навколо (x, y): квадрат size×size, не виходячи за межі грані. */
  const brush = (x: number, y: number): [number, number][] => {
    const { size, slim } = st.current;
    const r = rectAt(x, y, slim);
    const out: [number, number][] = [];
    const o = Math.floor((size - 1) / 2);
    for (let dy = 0; dy < size; dy++)
      for (let dx = 0; dx < size; dx++) {
        const px = x - o + dx;
        const py = y - o + dy;
        if (px < 0 || py < 0 || px > 63 || py > 63) continue;
        if (r && (px < r.x || py < r.y || px >= r.x + r.w || py >= r.y + r.h)) continue;
        out.push([px, py]);
      }
    return out;
  };

  const flood = (img: ImageData, x: number, y: number, r: Rect | undefined, c: RGBA) => {
    const box = r ?? { x: 0, y: 0, w: 64, h: 64 };
    const at = (px: number, py: number) => (py * 64 + px) * 4;
    const target = img.data.slice(at(x, y), at(x, y) + 4).join();
    if (target === c.join()) return;
    const stack: [number, number][] = [[x, y]];
    while (stack.length) {
      const [px, py] = stack.pop()!;
      if (px < box.x || py < box.y || px >= box.x + box.w || py >= box.y + box.h) continue;
      const i = at(px, py);
      if (img.data.slice(i, i + 4).join() !== target) continue;
      img.data.set(c, i);
      stack.push([px + 1, py], [px - 1, py], [px, py + 1], [px, py - 1]);
    }
  };

  const paintAt = (x: number, y: number) => {
    const { tool, color, mirrorOn, slim } = st.current;
    if (tool === "picker") return pick(x, y);
    const s = stroke.current;
    if (!s) return;
    const g = ctx();
    const img = g.getImageData(0, 0, 64, 64);
    const targets: [number, number][] = [[x, y]];
    if (mirrorOn) {
      const m = mirror(x, y, slim);
      if (m && (m[0] !== x || m[1] !== y)) targets.push(m);
    }
    if (tool === "fill") {
      for (const [tx, ty] of targets) flood(img, tx, ty, rectAt(tx, ty, slim), color);
    } else {
      const pixels = targets.flatMap(([tx, ty]) => brush(tx, ty));
      for (const [px, py] of pixels) {
        const i = (py * 64 + px) * 4;
        const k = py * 64 + px;
        if (tool === "pencil") img.data.set(color, i);
        else if (tool === "eraser") img.data.set([0, 0, 0, 0], i);
        else {
          // Тінь і шум — один раз на піксель за мазок, щоб не «випалювати» повторним проходом.
          if (s.visited.has(k) || img.data[i + 3] === 0) continue;
          s.visited.add(k);
          const f = tool === "darken" ? 0.88 : tool === "lighten" ? 1.12 : 1 + (Math.random() - 0.5) * 0.16;
          for (let ch = 0; ch < 3; ch++) img.data[i + ch] = Math.max(0, Math.min(255, Math.round(img.data[i + ch] * f + (tool === "lighten" ? 4 : 0))));
        }
      }
    }
    g.putImageData(img, 0, 0);
    bump();
  };

  /** Лінія від попередньої точки мазка, щоб швидкий рух не лишав дірок. */
  const paintLine = (x: number, y: number) => {
    const s = stroke.current;
    const last = s?.last;
    const tl = st.current.tool;
    if (!s || !last || tl === "fill" || tl === "picker") {
      paintAt(x, y);
    } else {
      // У 3D сусідні точки можуть бути на різних гранях — тоді лінія через розгортку безглузда.
      const r0 = rectAt(last[0], last[1], st.current.slim);
      const r1 = rectAt(x, y, st.current.slim);
      if (r0 !== r1) paintAt(x, y);
      else {
        let [x0, y0] = last;
        const dx = Math.abs(x - x0), dy = -Math.abs(y - y0);
        const sx = x0 < x ? 1 : -1, sy = y0 < y ? 1 : -1;
        let e = dx + dy;
        for (;;) {
          paintAt(x0, y0);
          if (x0 === x && y0 === y) break;
          const e2 = 2 * e;
          if (e2 >= dy) { e += dy; x0 += sx; }
          if (e2 <= dx) { e += dx; y0 += sy; }
        }
      }
    }
    if (s) s.last = [x, y];
  };

  const beginStroke = (x: number, y: number, alt: boolean) => {
    if (alt || st.current.tool === "picker") return pick(x, y);
    snapshot();
    stroke.current = { visited: new Set(), last: null };
    paintLine(x, y);
    if (st.current.tool === "pencil" || st.current.tool === "fill") {
      const h = hex(st.current.color);
      setRecent((r) => [h, ...r.filter((c) => c !== h)].slice(0, 12));
    }
  };
  const endStroke = () => {
    stroke.current = null;
  };

  // ---------------------------------------------------------------- клавіші
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el.closest?.("input, textarea")) return;
      const k = e.key.toLowerCase();
      if (e.ctrlKey && k === "z") {
        e.preventDefault();
        if (e.shiftKey) history(redo, undo);
        else history(undo, redo);
      } else if (e.ctrlKey && k === "y") {
        e.preventDefault();
        history(redo, undo);
      } else if (e.ctrlKey && k === "s") {
        e.preventDefault();
        doSave(false);
      } else if (!e.ctrlKey && !e.altKey) {
        const tl = TOOLS.find((x) => x.key === k);
        if (tl) setTool(tl.id);
        else if (k === "m") setMirrorOn((v) => !v);
        else if (k === "[") setSize((v) => Math.max(1, v - 1));
        else if (k === "]") setSize((v) => Math.min(4, v + 1));
        else if (k === "o") setLayer((v) => (v === "inner" ? "outer" : "inner"));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---------------------------------------------------------------- збереження
  const dataUrl = () => tex.current.toDataURL("image/png");
  const doSave = async (asNew: boolean) => {
    if (saving) return;
    setSaving(true);
    try {
      const s = await api.skinSavePng(dataUrl(), name.trim() || t("editor.new_name"), slim, asNew ? null : libId);
      setLibId(s.id);
      setDirty(false);
      window.dispatchEvent(new CustomEvent("lia:skins-changed"));
      toast("success", t("editor.saved"));
    } catch (e) {
      fail(e);
    } finally {
      setSaving(false);
    }
  };
  const doExport = async () => {
    const p = await save({ defaultPath: `${name.trim() || "skin"}.png`, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (p) api.skinExport(p, dataUrl()).then(() => toast("success", t("editor.exported"))).catch(fail);
  };

  const openSkin = (s: { name: string; data: string; slim: boolean; libId?: string | null }) => {
    setName(s.name);
    setSlim(s.slim);
    setLibId(s.libId ?? null);
    load(s.data).catch(fail);
    setOpenPicker(false);
  };

  const togglePart = (p: Part) =>
    setHidden((h) => {
      const n = new Set(h);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });

  return (
    <div className="page editor-page">
      <div className="page-head">
        <button className="btn ghost icon" onClick={async () => (!dirty || (await confirmDialog({ title: t("editor.leave"), danger: true }))) && back()} title={t("common.back")}>
          <ArrowLeft size={18} />
        </button>
        <input className="input editor-name" value={name} maxLength={32} onChange={(e) => setName(e.target.value)} />
        <div className="grow" />
        <button className="btn ghost icon" title={`${t("editor.undo")} (Ctrl+Z)`} onClick={() => history(undo, redo)}>
          <Undo2 size={17} />
        </button>
        <button className="btn ghost icon" title={`${t("editor.redo")} (Ctrl+Y)`} onClick={() => history(redo, undo)}>
          <Redo2 size={17} />
        </button>
        <button className="btn" onClick={() => setOpenPicker(true)}>
          <FolderOpen size={15} />
          {t("editor.open")}
        </button>
        <button className="btn" onClick={doExport}>
          <Download size={15} />
          PNG
        </button>
        {libId && (
          <button className="btn" disabled={saving} onClick={() => doSave(true)}>
            {t("editor.save_new")}
          </button>
        )}
        <button className="btn primary" disabled={saving || (!dirty && !!libId)} onClick={() => doSave(false)}>
          <Save size={15} />
          {t("common.save")}
        </button>
      </div>

      <div className="editor">
        <div className="editor-tools card">
          {TOOLS.map((tl) => (
            <button key={tl.id} className={`tool ${tool === tl.id ? "on" : ""}`} title={`${t(tl.label)} (${tl.key.toUpperCase()})`} onClick={() => setTool(tl.id)}>
              {tl.icon}
            </button>
          ))}
          <div className="tool-sep" />
          <button className={`tool ${mirrorOn ? "on" : ""}`} title={`${t("editor.mirror")} (M)`} onClick={() => setMirrorOn((v) => !v)}>
            <FlipHorizontal2 size={18} />
          </button>
          <button className={`tool ${grid ? "on" : ""}`} title={t("editor.grid")} onClick={() => setGrid((v) => !v)}>
            <Grid3x3 size={18} />
          </button>
          <div className="tool-sep" />
          {[1, 2, 3, 4].map((n) => (
            <button key={n} className={`tool size ${size === n ? "on" : ""}`} title={`${t("editor.size")} ${n} ([ ])`} onClick={() => setSize(n)}>
              <span style={{ width: 3 + n * 3, height: 3 + n * 3 }} />
            </button>
          ))}
        </div>

        <Viewport3D tex={tex.current} rev={rev} slim={slim} layer={layer} showOuter={showOuter} hidden={hidden} pose={pose} onDown={beginStroke} onMove={paintLine} onUp={endStroke} />

        <div className="editor-side">
          <Flat tex={tex.current} rev={rev} slim={slim} grid={grid} layer={layer} onDown={beginStroke} onMove={paintLine} onUp={endStroke} />

          <div className="card pad editor-panel">
            <div className="row" style={{ gap: 10 }}>
              <input type="color" className="color-well" value={hex(color)} onChange={(e) => setColor(parseHex(e.target.value)!)} />
              <input
                className="input mono"
                style={{ width: 110 }}
                key={hex(color)}
                defaultValue={hex(color)}
                onBlur={(e) => {
                  const c = parseHex(e.target.value);
                  if (c) setColor(c);
                }}
                onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
              />
              <span className="faint" style={{ fontSize: 12 }}>
                {t("editor.alt_pick")}
              </span>
            </div>
            <div className="swatches">
              {PALETTE.map((c) => (
                <button key={c} style={{ background: c }} onClick={() => setColor(parseHex(c)!)} title={c} />
              ))}
            </div>
            {recent.length > 0 && (
              <div className="swatches">
                {recent.map((c) => (
                  <button key={c} style={{ background: c }} onClick={() => setColor(parseHex(c)!)} title={c} />
                ))}
              </div>
            )}
          </div>

          <div className="card pad editor-panel">
            <div className="set-row">
              <span className="grow">{t("editor.layer")}</span>
              <Seg
                value={layer}
                onChange={setLayer}
                options={[
                  { value: "inner", label: t("editor.layer_inner") },
                  { value: "outer", label: t("editor.layer_outer") },
                ]}
              />
            </div>
            <div className="set-row">
              <span className="grow">{t("skins.model")}</span>
              <Seg
                value={slim ? "slim" : "classic"}
                onChange={(v) => {
                  setSlim(v === "slim");
                  setDirty(true);
                }}
                options={[
                  { value: "classic", label: t("acc.classic") },
                  { value: "slim", label: t("acc.slim") },
                ]}
              />
            </div>
            <div className="set-row">
              <span className="grow">{t("editor.show_outer")}</span>
              <Switch on={showOuter} onChange={setShowOuter} />
            </div>
            <div className="set-row">
              <span className="grow">{t("editor.pose")}</span>
              <Switch on={pose} onChange={setPose} />
            </div>
            <div className="parts">
              {PARTS.map((p) => (
                <button key={p} className={`chip ${hidden.has(p) ? "" : "on"}`} onClick={() => togglePart(p)}>
                  {t(PART_LABEL[p])}
                </button>
              ))}
            </div>
            <p className="hint">{t("editor.hint")}</p>
          </div>
        </div>
      </div>

      {openPicker && <OpenModal onClose={() => setOpenPicker(false)} onPick={openSkin} />}
    </div>
  );
}

// ---------------------------------------------------------------- 3D

function Viewport3D(props: {
  tex: HTMLCanvasElement;
  rev: number;
  slim: boolean;
  layer: Layer;
  showOuter: boolean;
  hidden: Set<Part>;
  pose: boolean;
  onDown: (x: number, y: number, alt: boolean) => void;
  onMove: (x: number, y: number) => void;
  onUp: () => void;
}) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<SkinViewer | null>(null);
  const cb = useRef(props);
  cb.current = props;

  useEffect(() => {
    const v = new SkinViewer({ canvas: canvas.current!, width: 400, height: 500, fov: 40, zoom: 0.9 });
    v.controls.enableZoom = true;
    v.controls.enablePan = false;
    v.loadSkin(props.tex, { model: props.slim ? "slim" : "default" });
    viewer.current = v;
    const ro = new ResizeObserver(([e]) => v.setSize(e.contentRect.width, e.contentRect.height));
    ro.observe(wrap.current!);
    return () => {
      ro.disconnect();
      v.dispose();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Текстура: копіюємо полотно редактора в текстуру моделі без перестворення.
  useEffect(() => {
    const v = viewer.current;
    if (!v) return;
    const g = v.skinCanvas.getContext("2d")!;
    g.clearRect(0, 0, v.skinCanvas.width, v.skinCanvas.height);
    g.drawImage(props.tex, 0, 0, v.skinCanvas.width, v.skinCanvas.height);
    if (v.playerObject.skin.map) v.playerObject.skin.map.needsUpdate = true;
  }, [props.rev, props.tex]);

  useEffect(() => {
    viewer.current?.loadSkin(props.tex, { model: props.slim ? "slim" : "default" });
  }, [props.slim]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const skin = viewer.current?.playerObject.skin;
    if (!skin) return;
    skin.setOuterLayerVisible(props.showOuter || props.layer === "outer");
    for (const p of PARTS) skin[p].visible = !props.hidden.has(p);
    // Розведені руки й ноги відкривають внутрішні грані.
    const a = props.pose ? 0.45 : 0;
    skin.rightArm.rotation.z = a;
    skin.leftArm.rotation.z = -a;
    skin.rightLeg.rotation.z = props.pose ? 0.12 : 0;
    skin.leftLeg.rotation.z = props.pose ? -0.12 : 0;
  }, [props.showOuter, props.layer, props.hidden, props.pose]);

  // Піксель текстури під курсором: перетин променя з видимими гранями обраного шару.
  const hit = (e: React.PointerEvent): [number, number] | null => {
    const v = viewer.current!;
    const r = canvas.current!.getBoundingClientRect();
    const ray = new Raycaster();
    ray.setFromCamera(new Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1), v.camera);
    const skin = v.playerObject.skin;
    const targets: Object3D[] = PARTS.filter((p) => !cb.current.hidden.has(p)).map((p) => (cb.current.layer === "outer" ? skin[p].outerLayer : skin[p].innerLayer));
    const h: Intersection | undefined = ray.intersectObjects(targets, true)[0];
    (window as any).__dbg = { h };
    if (!h?.uv || !h.face) return null;
    // Межа грані — середина між вершинами трикутника, щоб на ребрі не схопити сусідню грань.
    const uv = (h.object as unknown as { geometry: { attributes: { uv: { getX(i: number): number; getY(i: number): number } } } }).geometry.attributes.uv;
    const us = [h.face.a, h.face.b, h.face.c].map((i) => uv.getX(i) * 64);
    const vs = [h.face.a, h.face.b, h.face.c].map((i) => (1 - uv.getY(i)) * 64);
    const clamp = (n: number, lo: number, hi: number) => Math.min(Math.max(n, lo), hi);
    const x = clamp(Math.floor(h.uv.x * 64), Math.round(Math.min(...us)), Math.round(Math.max(...us)) - 1);
    const y = clamp(Math.floor((1 - h.uv.y) * 64), Math.round(Math.min(...vs)), Math.round(Math.max(...vs)) - 1);
    return [x, y];
  };

  const painting = useRef(false);
  return (
    <div
      ref={wrap}
      className="editor-3d card"
      onPointerDownCapture={(e) => {
        if (e.button !== 0) return;
        const p = hit(e);
        if (!p) return; // Порожнє місце — обертаємо камеру.
        viewer.current!.controls.enabled = false;
        painting.current = true;
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        cb.current.onDown(p[0], p[1], e.altKey);
      }}
      onPointerMove={(e) => {
        if (!painting.current) return;
        const p = hit(e);
        if (p) cb.current.onMove(p[0], p[1]);
      }}
      onPointerUp={() => {
        if (!painting.current) return;
        painting.current = false;
        viewer.current!.controls.enabled = true;
        cb.current.onUp();
      }}
    >
      <canvas ref={canvas} />
      <button
        className="btn ghost sm icon editor-reset"
        title={t("editor.reset_view")}
        onClick={() => {
          viewer.current!.resetCameraPose();
        }}
      >
        <RotateCcw size={15} />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------- розгортка 2D

function Flat(props: {
  tex: HTMLCanvasElement;
  rev: number;
  slim: boolean;
  grid: boolean;
  layer: Layer;
  onDown: (x: number, y: number, alt: boolean) => void;
  onMove: (x: number, y: number) => void;
  onUp: () => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [hover, setHover] = useState<[number, number] | null>(null);
  const painting = useRef(false);
  const SIZE = 512;

  useEffect(() => {
    const c = canvas.current!;
    const g = c.getContext("2d")!;
    const k = SIZE / 64;
    g.imageSmoothingEnabled = false;
    g.clearRect(0, 0, SIZE, SIZE);
    // Шахівниця під прозорістю, яскравіші — області обраного шару.
    for (const r of rects(props.slim)) {
      const active = r.layer === props.layer;
      for (let y = r.y; y < r.y + r.h; y++)
        for (let x = r.x; x < r.x + r.w; x++) {
          g.fillStyle = (x + y) % 2 ? (active ? "#2a3243" : "#1a1f2a") : active ? "#323b4f" : "#1e2430";
          g.fillRect(x * k, y * k, k, k);
        }
    }
    g.globalAlpha = 1;
    g.drawImage(props.tex, 0, 0, SIZE, SIZE);
    // Неактивний шар притлумлюємо, щоб було видно, куди малюєш.
    g.fillStyle = "rgba(11, 15, 22, 0.55)";
    for (const r of rects(props.slim)) if (r.layer !== props.layer) g.fillRect(r.x * k, r.y * k, r.w * k, r.h * k);
    if (props.grid) {
      g.strokeStyle = "rgba(255,255,255,0.06)";
      g.lineWidth = 1;
      g.beginPath();
      for (let i = 1; i < 64; i++) {
        g.moveTo(i * k + 0.5, 0);
        g.lineTo(i * k + 0.5, SIZE);
        g.moveTo(0, i * k + 0.5);
        g.lineTo(SIZE, i * k + 0.5);
      }
      g.stroke();
    }
    g.strokeStyle = "rgba(255,255,255,0.22)";
    for (const r of rects(props.slim)) g.strokeRect(r.x * k + 0.5, r.y * k + 0.5, r.w * k - 1, r.h * k - 1);
    if (hover) {
      g.strokeStyle = "#ffd23f";
      g.lineWidth = 2;
      g.strokeRect(hover[0] * k + 1, hover[1] * k + 1, k - 2, k - 2);
    }
  }, [props.rev, props.tex, props.slim, props.grid, props.layer, hover]);

  const at = (e: React.PointerEvent): [number, number] => {
    const r = canvas.current!.getBoundingClientRect();
    return [Math.min(63, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * 64))), Math.min(63, Math.max(0, Math.floor(((e.clientY - r.top) / r.height) * 64)))];
  };
  const r = hover ? rectAt(hover[0], hover[1], props.slim) : undefined;

  return (
    <div className="card editor-flat">
      <canvas
        ref={canvas}
        width={SIZE}
        height={SIZE}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          painting.current = true;
          (e.target as HTMLElement).setPointerCapture(e.pointerId);
          const [x, y] = at(e);
          props.onDown(x, y, e.altKey);
        }}
        onPointerMove={(e) => {
          const p = at(e);
          setHover(p);
          if (painting.current) props.onMove(p[0], p[1]);
        }}
        onPointerUp={() => {
          if (!painting.current) return;
          painting.current = false;
          props.onUp();
        }}
        onPointerLeave={() => setHover(null)}
      />
      <div className="editor-flat-info faint mono">
        {hover ? `${hover[0]}, ${hover[1]}` : " "}
        {r && ` · ${t(PART_LABEL[r.part])} · ${t(`editor.face.${r.face}` as Key)}`}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- відкрити

function OpenModal({ onClose, onPick }: { onClose: () => void; onPick: (s: { name: string; data: string; slim: boolean; libId?: string | null }) => void }) {
  const [lib, setLib] = useState<{ skins: LibSkin[]; defaults: LibSkin[] } | null>(null);
  useEffect(() => {
    api.skins().then(setLib).catch(fail);
  }, []);
  const fromFile = async () => {
    const f = await open({ multiple: false, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (typeof f !== "string") return;
    try {
      const s = await api.skinAddFile(f);
      onPick({ name: s.name, data: s.data, slim: s.slim, libId: s.id });
    } catch (e) {
      fail(e);
    }
  };
  const tile = (s: LibSkin, libId: string | null) => (
    <button key={s.id} className="editor-open-tile" onClick={() => onPick({ name: libId ? s.name : t("editor.new_name"), data: s.data, slim: s.slim, libId })}>
      <img src={s.data} alt="" />
      <span className="ellipsis">{s.name}</span>
    </button>
  );
  return (
    <Modal title={t("editor.open")} onClose={onClose} size="wide">
      <div className="row" style={{ gap: 8 }}>
        <button className="btn" onClick={() => onPick({ name: t("editor.new_name"), data: "", slim: false, libId: null })}>
          {t("editor.blank")}
        </button>
        <button className="btn" onClick={fromFile}>
          <FolderOpen size={15} />
          {t("skins.add_file")}
        </button>
      </div>
      {lib && lib.skins.length > 0 && (
        <>
          <h3 style={{ margin: "16px 0 8px" }}>{t("skins.mine")}</h3>
          <div className="editor-open-grid">{lib.skins.map((s) => tile(s, s.id))}</div>
        </>
      )}
      {lib && lib.defaults.length > 0 && (
        <>
          <h3 style={{ margin: "16px 0 8px" }}>{t("editor.templates")}</h3>
          <div className="editor-open-grid">{lib.defaults.map((s) => tile(s, null))}</div>
        </>
      )}
    </Modal>
  );
}
