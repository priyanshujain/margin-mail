import React from "react";
import ReactDOM from "react-dom/client";
import { isMacDesktop } from "./ipc";

// The three layers, in the order the design system stacks them: the tokens, the faces, then the
// base sheet the primitives and the screens layer on top of.
//
// Before the app, and that is the whole point of the order. A stylesheet imported by a component
// lands in the bundle where its module was first evaluated, so importing App first put every
// primitive's sheet above app.css and let `.panel` win ties against `.palette`.
import "./styles/tokens.css";
import "./styles/fonts.css";
import "./styles/app.css";

import App from "./App";
import { logNote } from "./api/log";

// Whatever the webview would otherwise say only to a developer console nobody has open. Same
// file as the engine's failures, so the log reads as one account of what went wrong.
window.addEventListener("error", (event) => {
  void logNote("ui", `uncaught: ${event.message} (${event.filename}:${event.lineno})`);
});
window.addEventListener("unhandledrejection", (event) => {
  void logNote("ui", `unhandled: ${String(event.reason)}`);
});

// The header's lane for the traffic lights, which only one platform draws over it. Written here
// rather than assumed by the stylesheet, so the first paint is already the right shape.
if (isMacDesktop) document.documentElement.setAttribute("data-traffic", "");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
