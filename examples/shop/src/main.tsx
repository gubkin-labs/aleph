import { AlephProvider, type Caller } from "@aleph-examples/inspector/react";
import { Analytics } from "@vercel/analytics/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";

const presets: Caller[] = [
  { label: "ada", userId: "ada" },
  { label: "bob", userId: "bob" },
];

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

createRoot(root).render(
  <StrictMode>
    <AlephProvider presets={presets}>
      <App />
    </AlephProvider>
    <Analytics />
  </StrictMode>,
);
