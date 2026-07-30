import type { Settings } from "@/types";

const media = window.matchMedia("(prefers-color-scheme: dark)");
let unsubscribe: (() => void) | null = null;

/** Applies a theme setting to the document root, resolving "system" via the OS preference. */
export function applyTheme(theme: Settings["theme"]) {
  unsubscribe?.();
  unsubscribe = null;

  if (theme === "system") {
    const sync = () => {
      document.documentElement.dataset.theme = media.matches ? "dark" : "light";
    };
    sync();
    media.addEventListener("change", sync);
    unsubscribe = () => media.removeEventListener("change", sync);
    return;
  }

  document.documentElement.dataset.theme = theme;
}
