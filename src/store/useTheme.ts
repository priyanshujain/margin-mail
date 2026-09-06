import { create } from "zustand";
import { applyTheme, initialTheme, type Theme } from "../theme";

interface ThemeState {
  theme: Theme;
  set: (theme: Theme) => void;
  toggle: () => void;
}

export const useTheme = create<ThemeState>((set, get) => ({
  theme: initialTheme(),
  set: (theme) => {
    applyTheme(theme);
    set({ theme });
  },
  toggle: () => get().set(get().theme === "dark" ? "light" : "dark"),
}));
