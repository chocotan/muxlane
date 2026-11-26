import { useCallback, useEffect, useState } from "react";

export type AppTheme =
  | "light"
  | "moonstone"
  | "paper"
  | "ash"
  | "sky"
  | "jade"
  | "sakura"
  | "dark"
  | "onedark"
  | "synthwave";

export interface ThemeDefinition {
  id: AppTheme;
  label: string;
  englishLabel: string;
  isDark: boolean;
  /** Compact square preview swatches (canvas, sidebar, accent, ink). */
  preview: {
    canvas: string;
    sidebar: string;
    accent: string;
    ink: string;
  };
}

/**
 * Ported directly from the Rust GPUI client (`crates/muxlane-app/src/theme.rs`).
 */
export const THEME_DEFINITIONS: readonly ThemeDefinition[] = [
  {
    id: "light",
    label: "雾白瓷",
    englishLabel: "Porcelain",
    isDark: false,
    preview: { canvas: "#faf9f6", sidebar: "#ece8e1", accent: "#3d6cd8", ink: "#242831" },
  },
  {
    id: "moonstone",
    label: "月白",
    englishLabel: "Moonstone",
    isDark: false,
    preview: { canvas: "#faf9fc", sidebar: "#f0eef5", accent: "#6650a4", ink: "#292633" },
  },
  {
    id: "paper",
    label: "纸暖",
    englishLabel: "Paper Warm",
    isDark: false,
    preview: { canvas: "#f4ecdf", sidebar: "#eadfce", accent: "#b35f2a", ink: "#3d3027" },
  },
  {
    id: "ash",
    label: "灰白",
    englishLabel: "Ash",
    isDark: false,
    preview: { canvas: "#f4f4f2", sidebar: "#e7e7e4", accent: "#3a3a36", ink: "#1a1a18" },
  },
  {
    id: "sky",
    label: "霁蓝",
    englishLabel: "Clear Sky",
    isDark: false,
    preview: { canvas: "#f2f7ff", sidebar: "#e3edfc", accent: "#2563eb", ink: "#1e293b" },
  },
  {
    id: "jade",
    label: "竹青",
    englishLabel: "Jade",
    isDark: false,
    preview: { canvas: "#eff8f2", sidebar: "#e0f0e6", accent: "#059669", ink: "#1f3529" },
  },
  {
    id: "sakura",
    label: "樱粉",
    englishLabel: "Sakura",
    isDark: false,
    preview: { canvas: "#fff2f5", sidebar: "#f9e3ea", accent: "#e11d48", ink: "#452530" },
  },
  {
    id: "dark",
    label: "墨渊",
    englishLabel: "Ink",
    isDark: true,
    preview: { canvas: "#181a1f", sidebar: "#21252b", accent: "#528bff", ink: "#d7dae0" },
  },
  {
    id: "onedark",
    label: "代码墨",
    englishLabel: "One Dark",
    isDark: true,
    preview: { canvas: "#282c34", sidebar: "#21252b", accent: "#61afef", ink: "#abb2bf" },
  },
  {
    id: "synthwave",
    label: "夜霓",
    englishLabel: "Synthwave",
    isDark: true,
    preview: { canvas: "#1b1029", sidebar: "#28163b", accent: "#e879f9", ink: "#f9eaff" },
  },
] as const;

const THEME_ID_SET = new Set<string>(THEME_DEFINITIONS.map((t) => t.id));

export function isValidTheme(value: unknown): value is AppTheme {
  return typeof value === "string" && THEME_ID_SET.has(value);
}

export function isDarkTheme(theme: AppTheme): boolean {
  return theme === "dark" || theme === "onedark" || theme === "synthwave";
}

export function applyThemeToDocument(theme: AppTheme): void {
  const root = document.documentElement;
  root.classList.remove("dark", "light");
  root.classList.add(isDarkTheme(theme) ? "dark" : "light");
  root.dataset.theme = theme;
}

const STORAGE_KEY = "muxlane.desktop.theme";

function readInitialTheme(): AppTheme {
  try {
    const saved = window.localStorage.getItem(STORAGE_KEY);
    if (isValidTheme(saved)) return saved;
  } catch {
    // ignore storage errors
  }
  return "dark";
}

export function useTheme() {
  const [theme, setThemeState] = useState<AppTheme>(() => {
    const initial = readInitialTheme();
    applyThemeToDocument(initial);
    return initial;
  });

  useEffect(() => {
    applyThemeToDocument(theme);
    try {
      window.localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // ignore storage errors
    }
  }, [theme]);

  const toggleTheme = useCallback(() => {
    setThemeState((prev) => (isDarkTheme(prev) ? "light" : "dark"));
  }, []);

  return { theme, setTheme: setThemeState, toggleTheme };
}
