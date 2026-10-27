// Розкладка текстури скіна 64×64: які прямокутники належать яким граням частин тіла.
// Потрібна редактору для дзеркала (ліва↔права половина) і заливки в межах грані.

export type Part = "head" | "body" | "rightArm" | "leftArm" | "rightLeg" | "leftLeg";
export type Layer = "inner" | "outer";
/** sideA — смуга ліворуч від передньої грані на розгортці, sideB — праворуч. */
export type Face = "top" | "bottom" | "sideA" | "front" | "sideB" | "back";

export interface Rect {
  part: Part;
  layer: Layer;
  face: Face;
  x: number;
  y: number;
  w: number;
  h: number;
}

export const PARTS: Part[] = ["head", "body", "rightArm", "leftArm", "rightLeg", "leftLeg"];

/** Розгортка коробки w×h×d з кутом (u, v) — та сама, що в skinview3d і в грі. */
function box(part: Part, layer: Layer, u: number, v: number, w: number, h: number, d: number): Rect[] {
  const r = (face: Face, x: number, y: number, rw: number, rh: number): Rect => ({ part, layer, face, x, y, w: rw, h: rh });
  return [
    r("top", u + d, v, w, d),
    r("bottom", u + d + w, v, w, d),
    r("sideA", u, v + d, d, h),
    r("front", u + d, v + d, w, h),
    r("sideB", u + d + w, v + d, d, h),
    r("back", u + 2 * d + w, v + d, w, h),
  ];
}

const cache = new Map<boolean, Rect[]>();

export function rects(slim: boolean): Rect[] {
  let out = cache.get(slim);
  if (out) return out;
  const aw = slim ? 3 : 4;
  out = [
    ...box("head", "inner", 0, 0, 8, 8, 8),
    ...box("head", "outer", 32, 0, 8, 8, 8),
    ...box("body", "inner", 16, 16, 8, 12, 4),
    ...box("body", "outer", 16, 32, 8, 12, 4),
    ...box("rightArm", "inner", 40, 16, aw, 12, 4),
    ...box("rightArm", "outer", 40, 32, aw, 12, 4),
    ...box("leftArm", "inner", 32, 48, aw, 12, 4),
    ...box("leftArm", "outer", 48, 48, aw, 12, 4),
    ...box("rightLeg", "inner", 0, 16, 4, 12, 4),
    ...box("rightLeg", "outer", 0, 32, 4, 12, 4),
    ...box("leftLeg", "inner", 16, 48, 4, 12, 4),
    ...box("leftLeg", "outer", 0, 48, 4, 12, 4),
  ];
  cache.set(slim, out);
  return out;
}

export function rectAt(x: number, y: number, slim: boolean): Rect | undefined {
  return rects(slim).find((r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
}

const SWAP_PART: Record<Part, Part> = { head: "head", body: "body", rightArm: "leftArm", leftArm: "rightArm", rightLeg: "leftLeg", leftLeg: "rightLeg" };
const SWAP_FACE: Record<Face, Face> = { top: "top", bottom: "bottom", front: "front", back: "back", sideA: "sideB", sideB: "sideA" };

/**
 * Дзеркальний піксель відносно вертикальної осі персонажа. Розгортка — це обхід коробки по колу,
 * тож дзеркало розвертає напрям обходу: у кожній грані x ↦ w-1-x, бічні грані міняються місцями.
 */
export function mirror(x: number, y: number, slim: boolean): [number, number] | null {
  const r = rectAt(x, y, slim);
  if (!r) return null;
  const t = rects(slim).find((o) => o.part === SWAP_PART[r.part] && o.layer === r.layer && o.face === SWAP_FACE[r.face]);
  if (!t) return null;
  return [t.x + (r.w - 1 - (x - r.x)), t.y + (y - r.y)];
}
