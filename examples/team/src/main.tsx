import { AlephProvider, type Caller } from "@aleph-examples/inspector/react";
import { Analytics } from "@vercel/analytics/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";

const presets: Caller[] = [
  { label: "owner", userId: "ada", orgs: { acme: "owner" } },
  { label: "admin", userId: "bob", orgs: { acme: "admin" } },
  { label: "member", userId: "cy", orgs: { acme: "member" } },
  { label: "API key", apiKey: { orgId: "acme", scopes: ["members:read"] } },
  { label: "outsider", userId: "eve", orgs: { other: "owner" } },
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
