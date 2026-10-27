import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource-variable/inter";
import "@fontsource-variable/unbounded";
import "@fontsource-variable/jetbrains-mono";
import "./styles/base.css";
import "./styles/ui.css";
import "./styles/pages.css";
import { App } from "./App";
import { fail, init } from "./lib/store";
import { scheduleUpdateChecks } from "./lib/updater";
import { initLia } from "./lib/lia";

// Без контекстного меню браузера: це застосунок, а не сторінка.
window.addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement).closest("input, textarea, .selectable")) e.preventDefault();
});

createRoot(document.getElementById("root")!).render(<App />);

init()
  .then(() => {
    initLia().catch(() => {});
    scheduleUpdateChecks();
  })
  .catch(fail)
  .finally(() => {
    // Вікно стартує прихованим, щоб не блимало білим до першого кадру.
    requestAnimationFrame(() => getCurrentWindow().show());
  });
