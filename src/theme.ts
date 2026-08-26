export const THEME_IDS = [
  "purple",
  "pink",
  "yellow",
  "deep-green",
  "sky",
  "white",
] as const;

export type AccentTheme = (typeof THEME_IDS)[number];

type ThemeRoot = {
  dataset: {
    accentTheme?: string;
  };
};

const DEFAULT_THEME: AccentTheme = "purple";
const themeIds = new Set<string>(THEME_IDS);

export function normalizeAccentTheme(value: unknown): AccentTheme {
  return typeof value === "string" && themeIds.has(value)
    ? (value as AccentTheme)
    : DEFAULT_THEME;
}

export function applyAccentTheme(
  value: unknown,
  root: ThemeRoot = document.documentElement,
): AccentTheme {
  const theme = normalizeAccentTheme(value);
  root.dataset.accentTheme = theme;
  return theme;
}

export function themeCssColor(
  variable: string,
  fallback: string,
  root: Element = document.documentElement,
): string {
  return getComputedStyle(root).getPropertyValue(variable).trim() || fallback;
}
