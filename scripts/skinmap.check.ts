// Перевірка розкладки скіна: node scripts/skinmap.check.ts
import assert from "node:assert/strict";
import { mirror, rectAt, rects } from "../src/lib/skinmap.ts";

for (const slim of [false, true]) {
  // Прямокутники не перетинаються.
  const seen = new Set<string>();
  for (const r of rects(slim))
    for (let y = r.y; y < r.y + r.h; y++)
      for (let x = r.x; x < r.x + r.w; x++) {
        assert.ok(!seen.has(`${x},${y}`), `перетин у ${x},${y}`);
        seen.add(`${x},${y}`);
      }
  // Дзеркало — інволюція і переводить кожен піксель у піксель тієї ж грані-пари.
  for (const key of seen) {
    const [x, y] = key.split(",").map(Number);
    const m = mirror(x, y, slim)!;
    assert.deepEqual(mirror(m[0], m[1], slim), [x, y], `дзеркало не інволюція в ${x},${y}`);
    assert.equal(rectAt(m[0], m[1], slim)!.layer, rectAt(x, y, slim)!.layer);
  }
}

// Ліве око Стіва (обличчя 8..15) дзеркалиться в праве; права рука → ліва, передня грань лишається передньою.
assert.deepEqual(mirror(9, 12, false), [14, 12]);
assert.deepEqual(mirror(44, 20, false), [39, 52]); // права рука, перед, x=0 → ліва рука, перед, x=3
assert.deepEqual(mirror(40, 20, false), [43, 52]); // бік A правої руки ↔ бік B лівої
console.log("skinmap: ok");
