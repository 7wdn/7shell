import { fileURLToPath, URL } from "node:url";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./apps/desktop/src", import.meta.url)),
    },
  },
  test: {
    environment: "happy-dom",
    include: ["apps/desktop/src/**/*.spec.ts", "packages/app-tests/**/*.test.ts"],
    testTimeout: 10_000,
    restoreMocks: true,
  },
});
