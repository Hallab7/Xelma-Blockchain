import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts", "tests/**/*.test.js"],
    environment: "node",
    globals: true,
    testTimeout: 30_000,
  },
});
