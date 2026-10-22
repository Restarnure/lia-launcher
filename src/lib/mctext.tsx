import type { CSSProperties, ReactNode } from "react";

// Текст Minecraft: компоненти чату (JSON) і коди форматування «§».

const COLORS: Record<string, string> = {
  black: "#000000",
  dark_blue: "#0000AA",
  dark_green: "#00AA00",
  dark_aqua: "#00AAAA",
  dark_red: "#AA0000",
  dark_purple: "#AA00AA",
  gold: "#FFAA00",
  gray: "#AAAAAA",
  dark_gray: "#555555",
  blue: "#5555FF",
  green: "#55FF55",
  aqua: "#55FFFF",
  red: "#FF5555",
  light_purple: "#FF55FF",
  yellow: "#FFFF55",
  white: "#FFFFFF",
};

const CODE_COLORS = "0123456789abcdef".split("");
const CODE_NAMES = Object.keys(COLORS);

interface Style {
  color?: string;
  bold?: boolean;
  italic?: boolean;
  underlined?: boolean;
  strikethrough?: boolean;
  obfuscated?: boolean;
}

interface Run {
  text: string;
  style: Style;
}

function legacy(text: string, base: Style, out: Run[]) {
  let style = { ...base };
  let buf = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if ((c === "§" || c === "&") && i + 1 < text.length && /[0-9a-fk-or]/i.test(text[i + 1])) {
      if (buf) out.push({ text: buf, style });
      buf = "";
      const code = text[++i].toLowerCase();
      const ci = CODE_COLORS.indexOf(code);
      if (ci >= 0) style = { color: COLORS[CODE_NAMES[ci]] };
      else if (code === "l") style = { ...style, bold: true };
      else if (code === "o") style = { ...style, italic: true };
      else if (code === "n") style = { ...style, underlined: true };
      else if (code === "m") style = { ...style, strikethrough: true };
      else if (code === "k") style = { ...style, obfuscated: true };
      else if (code === "r") style = { ...base };
    } else buf += c;
  }
  if (buf) out.push({ text: buf, style });
}

/* eslint-disable @typescript-eslint/no-explicit-any */
function walk(node: any, parent: Style, out: Run[]) {
  if (node == null) return;
  if (typeof node === "string") return legacy(node, parent, out);
  if (Array.isArray(node)) return node.forEach((n) => walk(n, parent, out));
  const style: Style = { ...parent };
  if (typeof node.color === "string") style.color = COLORS[node.color] ?? (node.color.startsWith("#") ? node.color : style.color);
  for (const k of ["bold", "italic", "underlined", "strikethrough", "obfuscated"] as const) if (typeof node[k] === "boolean") style[k] = node[k];
  if (typeof node.text === "string") legacy(node.text, style, out);
  if (typeof node.translate === "string" && !node.text) legacy(node.translate, style, out);
  if (Array.isArray(node.extra)) node.extra.forEach((n: any) => walk(n, style, out));
}

export function McText({ value, className }: { value: unknown; className?: string }) {
  const runs: Run[] = [];
  walk(value, {}, runs);
  const nodes: ReactNode[] = [];
  runs.forEach((r, i) => {
    const css: CSSProperties = {
      color: r.style.color,
      fontWeight: r.style.bold ? 700 : undefined,
      fontStyle: r.style.italic ? "italic" : undefined,
      textDecoration: [r.style.underlined && "underline", r.style.strikethrough && "line-through"].filter(Boolean).join(" ") || undefined,
    };
    const parts = r.text.split("\n");
    parts.forEach((p, j) => {
      if (j > 0) nodes.push(<br key={`${i}-br-${j}`} />);
      nodes.push(
        <span key={`${i}-${j}`} style={css} className={r.style.obfuscated ? "mc-obf" : undefined}>
          {p}
        </span>,
      );
    });
  });
  return <span className={className}>{nodes}</span>;
}
