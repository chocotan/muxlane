import { useCallback, useEffect, useState } from "react";
import type { SessionViewMode } from "../shared/protocol";
import {
  applyThemeToDocument,
  isDarkTheme,
  isValidTheme,
  type AppTheme,
} from "./useTheme";

export type TerminalFontFamily =
  | "SF Mono"
  | "JetBrains Mono"
  | "Noto Sans Mono"
  | "Liberation Mono"
  | "DejaVu Sans Mono";

export interface DesktopSettings {
  theme: AppTheme;
  uiScale: number;
  terminalFontSize: number;
  terminalFontFamily: TerminalFontFamily;
  defaultViewMode: SessionViewMode;
  notificationsEnabled: boolean;
  projectWorkspacesEnabled: boolean;
}

const STORAGE_KEY = "muxlane.desktop.settings.v2";
const LEGACY_V1_STORAGE_KEY = "muxlane.desktop.settings.v1";
const LEGACY_THEME_KEY = "muxlane.desktop.theme";

const DEFAULT_SETTINGS: DesktopSettings = {
  theme: "dark",
  uiScale: 100,
  terminalFontSize: 13,
  terminalFontFamily: "JetBrains Mono",
  defaultViewMode: "chat",
  notificationsEnabled: true,
  projectWorkspacesEnabled: true,
};

export const FONT_FAMILY_OPTIONS: Array<{ value: TerminalFontFamily; css: string }> = [
  {
    value: "JetBrains Mono",
    css: '"JetBrains Mono", "SF Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace',
  },
  {
    value: "SF Mono",
    css: '"SF Mono", "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
  },
  {
    value: "Noto Sans Mono",
    css: '"Noto Sans Mono", "JetBrains Mono", ui-monospace, monospace',
  },
  {
    value: "Liberation Mono",
    css: '"Liberation Mono", "DejaVu Sans Mono", ui-monospace, monospace',
  },
  {
    value: "DejaVu Sans Mono",
    css: '"DejaVu Sans Mono", "Liberation Mono", ui-monospace, monospace',
  },
];

export function resolveTerminalFontCss(family: TerminalFontFamily): string {
  return (
    FONT_FAMILY_OPTIONS.find((opt) => opt.value === family)?.css ??
    FONT_FAMILY_OPTIONS[0].css
  );
}

function loadSettings(): DesktopSettings {
  try {
    const rawV2 = window.localStorage.getItem(STORAGE_KEY);
    if (rawV2) {
      const parsed = JSON.parse(rawV2) as Partial<DesktopSettings>;
      return {
        theme: isValidTheme(parsed.theme) ? parsed.theme : DEFAULT_SETTINGS.theme,
        uiScale:
          typeof parsed.uiScale === "number" && parsed.uiScale >= 80 && parsed.uiScale <= 130
            ? Math.round(parsed.uiScale)
            : DEFAULT_SETTINGS.uiScale,
        terminalFontSize:
          typeof parsed.terminalFontSize === "number" &&
          parsed.terminalFontSize >= 10 &&
          parsed.terminalFontSize <= 20
            ? Math.round(parsed.terminalFontSize)
            : DEFAULT_SETTINGS.terminalFontSize,
        terminalFontFamily: FONT_FAMILY_OPTIONS.some((f) => f.value === parsed.terminalFontFamily)
          ? (parsed.terminalFontFamily as TerminalFontFamily)
          : DEFAULT_SETTINGS.terminalFontFamily,
        defaultViewMode:
          parsed.defaultViewMode === "chat" ||
          parsed.defaultViewMode === "terminal" ||
          parsed.defaultViewMode === "split"
            ? parsed.defaultViewMode
            : DEFAULT_SETTINGS.defaultViewMode,
        notificationsEnabled:
          typeof parsed.notificationsEnabled === "boolean"
            ? parsed.notificationsEnabled
            : DEFAULT_SETTINGS.notificationsEnabled,
        projectWorkspacesEnabled:
          typeof parsed.projectWorkspacesEnabled === "boolean"
            ? parsed.projectWorkspacesEnabled
            : DEFAULT_SETTINGS.projectWorkspacesEnabled,
      };
    }
    // Migrate v1 settings: preserve theme/font/scale, adopt new 'chat' default view mode.
    const rawV1 = window.localStorage.getItem(LEGACY_V1_STORAGE_KEY);
    if (rawV1) {
      const parsed = JSON.parse(rawV1) as Partial<DesktopSettings>;
      return {
        theme: isValidTheme(parsed.theme) ? parsed.theme : DEFAULT_SETTINGS.theme,
        uiScale:
          typeof parsed.uiScale === "number" && parsed.uiScale >= 80 && parsed.uiScale <= 130
            ? Math.round(parsed.uiScale)
            : DEFAULT_SETTINGS.uiScale,
        terminalFontSize:
          typeof parsed.terminalFontSize === "number" &&
          parsed.terminalFontSize >= 10 &&
          parsed.terminalFontSize <= 20
            ? Math.round(parsed.terminalFontSize)
            : DEFAULT_SETTINGS.terminalFontSize,
        terminalFontFamily: FONT_FAMILY_OPTIONS.some((f) => f.value === parsed.terminalFontFamily)
          ? (parsed.terminalFontFamily as TerminalFontFamily)
          : DEFAULT_SETTINGS.terminalFontFamily,
        defaultViewMode: "chat",
        notificationsEnabled:
          typeof parsed.notificationsEnabled === "boolean"
            ? parsed.notificationsEnabled
            : DEFAULT_SETTINGS.notificationsEnabled,
        projectWorkspacesEnabled: DEFAULT_SETTINGS.projectWorkspacesEnabled,
      };
    }
    const legacyTheme = window.localStorage.getItem(LEGACY_THEME_KEY);
    if (isValidTheme(legacyTheme)) {
      return { ...DEFAULT_SETTINGS, theme: legacyTheme };
    }
  } catch {
    // ignore storage errors
  }
  return DEFAULT_SETTINGS;
}

export function useSettings() {
  const [settings, setSettings] = useState<DesktopSettings>(loadSettings);

  useEffect(() => {
    applyThemeToDocument(settings.theme);
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
      window.localStorage.setItem(LEGACY_THEME_KEY, settings.theme);
    } catch {
      // ignore
    }
  }, [settings]);

  useEffect(() => {
    try {
      window.muxlane?.setZoomFactor?.(settings.uiScale / 100);
    } catch {
      // ignore in non-Electron test environments
    }
  }, [settings.uiScale]);

  const updateSettings = useCallback((patch: Partial<DesktopSettings>) => {
    setSettings((prev) => ({ ...prev, ...patch }));
  }, []);

  const toggleTheme = useCallback(() => {
    setSettings((prev) => ({ ...prev, theme: isDarkTheme(prev.theme) ? "light" : "dark" }));
  }, []);

  const resetSettings = useCallback(() => {
    setSettings(DEFAULT_SETTINGS);
  }, []);

  return {
    settings,
    updateSettings,
    toggleTheme,
    resetSettings,
  };
}
