import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { isMacDesktop } from "./ipc";
import "./styles/tokens.css";
import "./styles/fonts.css";
import "./styles/app.css";

// The header's lane for the traffic lights, which only one platform draws over it. Written here
// rather than assumed by the stylesheet, so the first paint is already the right shape.
if (isMacDesktop) document.documentElement.setAttribute("data-traffic", "");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
