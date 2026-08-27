import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  THEME_IDS,
  applyAccentTheme,
  normalizeAccentTheme,
} from "../src/theme.ts";

const themeCss = readFileSync(new URL("../src/theme.css", import.meta.url), "utf8");
const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const memoHtml = readFileSync(new URL("../memo.html", import.meta.url), "utf8");
const vaultHtml = readFileSync(new URL("../vault.html", import.meta.url), "utf8");
const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const memoSource = readFileSync(new URL("../src/memo.ts", import.meta.url), "utf8");
const vaultSource = readFileSync(new URL("../src/vault.ts", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const libSource = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const settingsSource = readFileSync(
  new URL("../src-tauri/src/settings.rs", import.meta.url),
  "utf8",
);

function paletteBlock(theme: string): string {
  const selector = `:root[data-accent-theme="${theme}"]`;
  const start = themeCss.indexOf(selector);
  assert.notEqual(start, -1, `${selector} must exist`);
  return themeCss.slice(themeCss.indexOf("{", start) + 1, themeCss.indexOf("}", start));
}

function paletteRgb(theme: string, token: string): [number, number, number] {
  const match = paletteBlock(theme).match(
    new RegExp(`--${token}-rgb\\s*:\\s*(\\d+)\\s*,\\s*(\\d+)\\s*,\\s*(\\d+)`),
  );
  assert.ok(match, `${theme} needs --${token}-rgb`);
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function luminance(rgb: [number, number, number]): number {
  const [red, green, blue] = rgb.map((value) => {
    const channel = value / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrast(a: [number, number, number], b: [number, number, number]): number {
  const lighter = Math.max(luminance(a), luminance(b));
  const darker = Math.min(luminance(a), luminance(b));
  return (lighter + 0.05) / (darker + 0.05);
}

function colorDistance(
  a: [number, number, number],
  b: [number, number, number],
): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

test("normalizes unknown accent themes to the current purple default", () => {
  assert.deepEqual(THEME_IDS, ["purple", "pink", "yellow", "deep-green", "sky", "white"]);
  for (const theme of THEME_IDS) assert.equal(normalizeAccentTheme(theme), theme);
  for (const value of [undefined, null, "", "green", 1]) {
    assert.equal(normalizeAccentTheme(value), "purple");
  }
});

test("applies a normalized theme through data-accent-theme", () => {
  const root: { dataset: { accentTheme?: string } } = { dataset: {} };

  assert.equal(applyAccentTheme("sky", root), "sky");
  assert.equal(root.dataset.accentTheme, "sky");
  assert.equal(applyAccentTheme("not-a-theme", root), "purple");
  assert.equal(root.dataset.accentTheme, "purple");
});

test("defines all six complete dark palettes and the shared derived colors", () => {
  const paletteTokens = [
    "bg",
    "panel",
    "panel-2",
    "text",
    "muted",
    "border",
    "track",
    "separator",
    "accent",
    "accent-alt",
    "on-accent",
    "active-text",
  ];

  for (const theme of THEME_IDS) {
    const block = paletteBlock(theme);
    for (const token of paletteTokens) {
      assert.match(block, new RegExp(`--${token}-rgb\\s*:`), `${theme} needs --${token}-rgb`);
    }
  }

  assert.match(themeCss, /--bg:\s*rgba\(var\(--bg-rgb\), var\(--bg-alpha, 1\)\)/);
  assert.match(themeCss, /--panel:\s*rgba\(var\(--panel-rgb\), var\(--bg-alpha, 1\)\)/);
  assert.match(themeCss, /--panel-2:\s*rgba\(var\(--panel-2-rgb\), var\(--bg-alpha, 1\)\)/);
  assert.match(themeCss, /--text:\s*rgba\(var\(--text-rgb\), var\(--fg-alpha, 1\)\)/);
  assert.match(themeCss, /--muted:\s*rgba\(var\(--muted-rgb\), var\(--fg-alpha, 1\)\)/);
  assert.match(themeCss, /--accent:\s*rgb\(var\(--accent-rgb\)\)/);
  assert.match(themeCss, /--accent-alt:\s*rgb\(var\(--accent-alt-rgb\)\)/);
  assert.match(themeCss, /--on-accent:\s*rgb\(var\(--on-accent-rgb\)\)/);
  assert.match(themeCss, /--active-text:\s*rgba\(var\(--active-text-rgb\), var\(--fg-alpha, 1\)\)/);

  assert.match(themeCss, /--bg-rgb:\s*21, 21, 28;/);
  assert.match(themeCss, /--panel-rgb:\s*29, 29, 39;/);
  assert.match(themeCss, /--panel-2-rgb:\s*38, 38, 51;/);
  assert.match(themeCss, /--text-rgb:\s*240, 238, 248;/);
  assert.match(themeCss, /--muted-rgb:\s*159, 155, 176;/);
  assert.deepEqual(paletteRgb("pink", "accent"), [249, 168, 212]);
  assert.deepEqual(paletteRgb("yellow", "accent"), [253, 230, 138]);
  assert.deepEqual(paletteRgb("purple", "accent"), [196, 181, 253]);
  assert.deepEqual(paletteRgb("sky", "accent"), [186, 230, 253]);
  assert.deepEqual(paletteRgb("white", "accent"), [248, 250, 252]);
  assert.deepEqual(
    paletteRgb("deep-green", "accent"),
    [52, 211, 153],
    "deep green keeps its saturated identity",
  );
});

test("keeps text, accents, and accent buttons readable in every palette", () => {
  for (const theme of THEME_IDS) {
    const panel = paletteRgb(theme, "panel");
    const accent = paletteRgb(theme, "accent");
    assert.ok(
      contrast(paletteRgb(theme, "text"), panel) >= 4.5,
      `${theme} text must reach 4.5:1 against its panel`,
    );
    assert.ok(
      contrast(paletteRgb(theme, "muted"), panel) >= 4.5,
      `${theme} muted text must reach 4.5:1 against its panel`,
    );
    assert.ok(
      contrast(accent, panel) >= 3,
      `${theme} accent controls must reach 3:1 against their panel`,
    );
    assert.ok(
      contrast(paletteRgb(theme, "on-accent"), accent) >= 4.5,
      `${theme} accent buttons must keep 4.5:1 text contrast`,
    );
  }
});

test("keeps themed monitor series and warning fills distinguishable", () => {
  assert.deepEqual(paletteRgb("yellow", "monitor-dsk"), [196, 181, 253]);
  assert.deepEqual(paletteRgb("deep-green", "monitor-mem"), [196, 181, 253]);
  assert.deepEqual(paletteRgb("sky", "monitor-net"), [249, 168, 212]);

  const defaults = {
    mem: [74, 222, 128],
    dsk: [250, 204, 21],
    net: [96, 165, 250],
  } as const;
  for (const theme of THEME_IDS) {
    const block = paletteBlock(theme);
    const series: Array<[string, [number, number, number]]> = [
      ["cpu", paletteRgb(theme, "accent")],
      [
        "mem",
        block.includes("--monitor-mem-rgb")
          ? paletteRgb(theme, "monitor-mem")
          : [...defaults.mem],
      ],
      [
        "dsk",
        block.includes("--monitor-dsk-rgb")
          ? paletteRgb(theme, "monitor-dsk")
          : [...defaults.dsk],
      ],
      [
        "net",
        block.includes("--monitor-net-rgb")
          ? paletteRgb(theme, "monitor-net")
          : [...defaults.net],
      ],
    ];
    for (let left = 0; left < series.length; left += 1) {
      for (let right = left + 1; right < series.length; right += 1) {
        assert.ok(
          colorDistance(series[left][1], series[right][1]) >= 50,
          `${theme} ${series[left][0]} and ${series[right][0]} must stay visually distinct`,
        );
      }
    }
  }

  assert.match(
    stylesSource,
    /\.mon-row \.bar-fill\.mon-fill-mem\s*{[^}]*--monitor-mem-rgb/s,
  );
  assert.match(
    stylesSource,
    /\.mon-row \.bar-fill\.mon-fill-dsk\s*{[^}]*--monitor-dsk-rgb/s,
  );
  assert.match(
    stylesSource,
    /\.mon-row \.bar-fill\.mon-fill-net\s*{[^}]*--monitor-net-rgb/s,
  );
  assert.match(
    stylesSource,
    /\.bar-fill\.warn\s*{[^}]*repeating-linear-gradient/s,
    "warning usage must retain a non-color signal when the selected accent is yellow",
  );
});

test("wires the native setting into every themed window", () => {
  assert.match(indexHtml, /src\/theme\.css/);
  assert.match(memoHtml, /src\/theme\.css/);
  assert.match(vaultHtml, /src\/theme\.css/);
  assert.match(themeCss, /data-accent-theme="purple"/);
  assert.match(settingsSource, /get_accent_theme|load_accent_theme/);
  assert.match(libSource, /get_accent_theme/);
  assert.match(libSource, /accent-theme-changed/);

  for (const source of [mainSource, memoSource, vaultSource]) {
    assert.match(source, /get_accent_theme/);
    assert.match(source, /accent-theme-changed/);
    assert.match(source, /applyAccentTheme/);
  }

  assert.match(indexHtml, /stroke="currentColor"/);
  assert.doesNotMatch(mainSource, /rgba\(167,\s*139,\s*250,\s*(?:0\.9|0\.15)\)/);
});
