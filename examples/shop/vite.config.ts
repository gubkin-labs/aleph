import { aleph } from "@aleph-examples/inspector/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    react(),
    aleph({
      app: "./app.ts",
      port: 4312,
      // The dev engine has no real payment provider: charges succeed until you flip it in the UI.
      mocks: [
        { integration: "payments", operation: "charge", response: { chargeId: "ch_dev_ok" } },
      ],
    }),
  ],
});
