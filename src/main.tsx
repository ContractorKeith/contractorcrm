import "@fontsource/barlow/latin-400.css";
import "@fontsource/barlow/latin-500.css";
import "@fontsource/barlow-condensed/latin-600.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("ContractorCRM root element is missing");

document.documentElement.dataset.platform = /Mac/.test(navigator.platform) ? "macos" : "other";

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
