// Ізометричні піксельні блоки для іконок збірок. Текстури 16×16 генеруються кодом —
// жодних файлів Mojang, але впізнавано «по-майнкрафтівськи».

type RGB = [number, number, number];
type Painter = (x: number, y: number, r: () => number) => RGB;

function hex(h: string): RGB {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function shade([r, g, b]: RGB, k: number): RGB {
  return [Math.min(255, r * k), Math.min(255, g * k), Math.min(255, b * k)];
}

/** Вибір кольору з палітри з невеликим шумом. */
function noise(pal: string[]): Painter {
  const p = pal.map(hex);
  return (_x, _y, r) => p[Math.floor(r() * p.length)];
}

const grassTop = noise(["#5f9e35", "#6aac3b", "#58932f", "#73b545", "#64a438"]);
const dirt = noise(["#866043", "#79553a", "#6c4a32", "#8f6a4c", "#7d5940"]);
const stone = noise(["#7d7d7d", "#747474", "#868686", "#6e6e6e", "#7a7a7a"]);

function planks(base: string, dark: string, light: string): Painter {
  const b = hex(base),
    d = hex(dark),
    l = hex(light);
  return (x, y, r) => {
    if (y % 4 === 3) return d;
    const seam = (Math.floor(y / 4) % 2 === 0 ? 7 : 15) === x;
    if (seam) return d;
    return r() < 0.18 ? l : r() < 0.12 ? d : b;
  };
}

function ore(base: Painter, gem: string[], spots: [number, number][]): Painter {
  const g = gem.map(hex);
  return (x, y, r) => {
    for (const [sx, sy] of spots) {
      if (x >= sx && x < sx + 2 && y >= sy && y < sy + 2) return g[(x + y) % g.length];
    }
    return base(x, y, r);
  };
}

function solid(pal: string[], frame?: string): Painter {
  const n = noise(pal);
  const f = frame ? hex(frame) : null;
  return (x, y, r) => (f && (x === 0 || y === 0 || x === 15 || y === 15) ? f : n(x, y, r));
}

interface BlockDef {
  top: Painter;
  side: Painter;
}

const spots: [number, number][] = [[2, 3], [9, 2], [5, 8], [12, 9], [3, 12], [10, 13]];

export const BLOCKS: Record<string, BlockDef> = {
  grass: {
    top: grassTop,
    side: (x, y, r) => (y < 3 || (y === 3 && r() < 0.5) ? grassTop(x, y, r) : dirt(x, y, r)),
  },
  dirt: { top: dirt, side: dirt },
  stone: { top: stone, side: stone },
  cobblestone: {
    top: (x, y, r) => (((x * 7 + y * 13) % 11 < 2 || r() < 0.08) ? hex("#555555") : stone(x, y, r)),
    side: (x, y, r) => (((x * 7 + y * 13) % 11 < 2 || r() < 0.08) ? hex("#555555") : stone(x, y, r)),
  },
  planks: { top: planks("#a2814e", "#7a5f37", "#b7955c"), side: planks("#a2814e", "#7a5f37", "#b7955c") },
  log: {
    top: (x, y) => {
      const d = Math.max(Math.abs(x - 7.5), Math.abs(y - 7.5));
      return d > 6.5 ? hex("#6b5233") : Math.floor(d) % 2 ? hex("#b49159") : hex("#a07f4c");
    },
    side: (x, _y, r) => (x % 4 === 0 ? hex("#4f3c24") : r() < 0.2 ? hex("#5e4a2e") : hex("#6b5233")),
  },
  sand: { top: noise(["#dbd3a0", "#d6cc95", "#e0d8a8", "#cfc68d"]), side: noise(["#dbd3a0", "#d6cc95", "#e0d8a8", "#cfc68d"]) },
  diamond: { top: ore(stone, ["#5decf5", "#2fc5d0", "#a8fbff"], spots), side: ore(stone, ["#5decf5", "#2fc5d0", "#a8fbff"], spots) },
  emerald: { top: ore(stone, ["#41f384", "#17c75a", "#a5ffc9"], spots), side: ore(stone, ["#41f384", "#17c75a", "#a5ffc9"], spots) },
  gold: { top: solid(["#fde84a", "#f6d92e", "#ffef7a"], "#d9a521"), side: solid(["#fde84a", "#f6d92e", "#ffef7a"], "#d9a521") },
  iron: { top: solid(["#e6e6e6", "#dcdcdc", "#f0f0f0"], "#b3b3b3"), side: solid(["#e6e6e6", "#dcdcdc", "#f0f0f0"], "#b3b3b3") },
  lapis: { top: solid(["#2552b8", "#1e4aa8", "#2f62cc"], "#163a86"), side: solid(["#2552b8", "#1e4aa8", "#2f62cc"], "#163a86") },
  redstone: { top: solid(["#b81d0b", "#a51708", "#cc2a12"], "#7d1306"), side: solid(["#b81d0b", "#a51708", "#cc2a12"], "#7d1306") },
  tnt: {
    top: (x, y, r) => (x > 4 && x < 11 && y > 4 && y < 11 ? hex("#3a3a3a") : r() < 0.2 ? hex("#c9372a") : hex("#db4535")),
    side: (x, y, r) => {
      if (y >= 6 && y <= 9) return (x + y) % 5 === 0 ? hex("#222") : hex("#e9e9e2");
      return x % 4 === 0 ? hex("#a72a1e") : r() < 0.15 ? hex("#c9372a") : hex("#db4535");
    },
  },
  bookshelf: {
    top: planks("#a2814e", "#7a5f37", "#b7955c"),
    side: (x, y, r) => {
      if (y === 0 || y === 15 || y === 7 || y === 8) return hex("#a2814e");
      const books = ["#7a1f1f", "#2a4d8f", "#2f6b2f", "#8a6a21", "#5b2a6e", "#a14c1b"];
      return x % 3 === 2 ? hex("#2a1f12") : hex(books[(x * 3 + (y > 7 ? 2 : 0) + Math.floor(r() * 1.2)) % books.length]);
    },
  },
  crafting: {
    top: (x, y, r) => (x === 0 || y === 0 || x === 15 || y === 15 ? hex("#6b5233") : x % 5 === 0 || y % 5 === 0 ? hex("#5b432a") : planks("#b18c58", "#8d6c40", "#c49d65")(x, y, r)),
    side: (x, y, r) => (y < 3 ? hex("#6b5233") : (x > 2 && x < 7 && y > 5 && y < 11) || (x > 9 && x < 13 && y > 6 && y < 12) ? hex("#4b4b4b") : planks("#a2814e", "#7a5f37", "#b7955c")(x, y, r)),
  },
  netherrack: { top: noise(["#7f3333", "#6f2a2a", "#8c3b3b", "#612323"]), side: noise(["#7f3333", "#6f2a2a", "#8c3b3b", "#612323"]) },
  obsidian: { top: noise(["#15101f", "#1d1530", "#271d40", "#100c18", "#3b2a5e"]), side: noise(["#15101f", "#1d1530", "#271d40", "#100c18", "#3b2a5e"]) },
  ice: { top: noise(["#8fb4f7", "#9cbdf9", "#a8c6fa", "#86abf2"]), side: noise(["#8fb4f7", "#9cbdf9", "#a8c6fa", "#86abf2"]) },
  pumpkin: {
    top: (x, y, r) => (x > 6 && x < 9 && y > 6 && y < 9 ? hex("#5a7a2a") : r() < 0.2 ? hex("#c4741c") : hex("#d98a21")),
    side: (x, y, r) => {
      const eye = (y === 5 || y === 6) && (x === 4 || x === 5 || x === 10 || x === 11);
      const mouth = y >= 9 && y <= 11 && x >= 4 && x <= 11 && !(y === 9 && (x === 6 || x === 9));
      if (eye || mouth) return hex("#3b1d05");
      return x % 4 === 0 ? hex("#b8691a") : r() < 0.15 ? hex("#c4741c") : hex("#d98a21");
    },
  },
  amethyst: { top: noise(["#8d62c9", "#a47be0", "#7a51b5", "#c49af0"]), side: noise(["#8d62c9", "#a47be0", "#7a51b5", "#c49af0"]) },
  sculk: {
    top: (x, y, r) => (((x * 5 + y * 3) % 9 === 0) ? hex("#29dfeb") : noise(["#0d1e26", "#0a2a33", "#112830", "#071820"])(x, y, r)),
    side: (x, y, r) => (((x * 5 + y * 3) % 9 === 0) ? hex("#29dfeb") : noise(["#0d1e26", "#0a2a33", "#112830", "#071820"])(x, y, r)),
  },
  prismarine: { top: noise(["#5fa59a", "#63b0a3", "#4d8f85", "#78c2b2"]), side: noise(["#5fa59a", "#63b0a3", "#4d8f85", "#78c2b2"]) },
  shotam: {
    top: (x, y, r) => (x === 0 || y === 0 || x === 15 || y === 15 ? hex("#d9a91b") : r() < 0.2 ? hex("#ffdd66") : hex("#ffd23f")),
    side: (x, y, r) => (x === 0 || y === 0 || x === 15 || y === 15 ? hex("#1a3f99") : r() < 0.2 ? hex("#3d7bff") : hex("#2f66e0")),
  },
};

export const BLOCK_KEYS = Object.keys(BLOCKS).filter((k) => k !== "shotam");

function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return (s >>> 0) / 4294967296;
  };
}

function texture(p: Painter, seed: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = 16;
  const g = c.getContext("2d")!;
  const img = g.createImageData(16, 16);
  const r = rng(seed);
  for (let y = 0; y < 16; y++)
    for (let x = 0; x < 16; x++) {
      const [R, G, B] = p(x, y, r);
      const i = (y * 16 + x) * 4;
      img.data[i] = R;
      img.data[i + 1] = G;
      img.data[i + 2] = B;
      img.data[i + 3] = 255;
    }
  g.putImageData(img, 0, 0);
  return c;
}

function tinted(src: HTMLCanvasElement, k: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = 16;
  const g = c.getContext("2d")!;
  g.drawImage(src, 0, 0);
  const img = g.getImageData(0, 0, 16, 16);
  for (let i = 0; i < img.data.length; i += 4) {
    const [r, gg, b] = shade([img.data[i], img.data[i + 1], img.data[i + 2]], k);
    img.data[i] = r;
    img.data[i + 1] = gg;
    img.data[i + 2] = b;
  }
  g.putImageData(img, 0, 0);
  return c;
}

const cache = new Map<string, string>();

/** PNG (data URL) ізометричного блока заданого розміру. */
export function blockImage(key: string, size = 128): string {
  const id = `${key}@${size}`;
  const hit = cache.get(id);
  if (hit) return hit;
  const def = BLOCKS[key] ?? BLOCKS.grass;
  const seed = [...key].reduce((a, c) => a * 31 + c.charCodeAt(0), 7);
  const top = texture(def.top, seed);
  const sideBase = texture(def.side, seed + 1);
  const left = tinted(sideBase, 0.82);
  const right = tinted(sideBase, 0.62);

  const c = document.createElement("canvas");
  c.width = c.height = size;
  const g = c.getContext("2d")!;
  g.imageSmoothingEnabled = false;
  const w = size * 0.86;
  const cx = size / 2;
  const a = w / 2 / 16; // крок по x на піксель текстури
  const b = a * 0.577; // ізометрія 30°
  const hs = w * 0.58 / 16; // висота бічної грані на піксель
  const topY = (size - (b * 32 + hs * 16)) / 2;
  const midY = topY + b * 16;
  const centerY = topY + b * 32;

  g.setTransform(a, b, -a, b, cx, topY);
  g.drawImage(top, 0, 0);
  g.setTransform(a, b, 0, hs, cx - a * 16, midY);
  g.drawImage(left, 0, 0);
  g.setTransform(a, -b, 0, hs, cx, centerY);
  g.drawImage(right, 0, 0);
  g.setTransform(1, 0, 0, 1, 0, 0);

  const url = c.toDataURL();
  cache.set(id, url);
  return url;
}
