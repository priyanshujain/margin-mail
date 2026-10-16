import { create } from "zustand";
import { applyTheme, hasThemeChoice, initialTheme, syncWindowTheme, type Theme } from "../theme";

interface ThemeState {
  theme: Theme;
  set: (theme: Theme) => void;
  toggle: () => void;
}

const startTheme = initialTheme();
syncWindowTheme(hasThemeChoice() ? startTheme : null);

export const useTheme = create<ThemeState>((set, get) => ({
  theme: startTheme,
  set: (theme) => {
    applyTheme(theme);
    set({ theme });
  },
  toggle: () => get().set(get().theme === "dark" ? "light" : "dark"),
}));
