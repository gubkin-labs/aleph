import { aleph } from "@aleph-examples/inspector/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), aleph({ app: "./app.ts", port: 4310 })],
});
