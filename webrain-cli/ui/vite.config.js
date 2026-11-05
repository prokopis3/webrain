import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { rmSync } from "node:fs";
import { fileURLToPath } from "node:url";

// `index.html` only exists to give Vite an entry; the real shell is
// `../assets/chat.html`, served by the Rust binary. Drop the copy Vite emits so
// the build is idempotent and leaves exactly chat.js + chat.css behind.
const dropShell = {
  name: "drop-shell",
  closeBundle() {
    rmSync(fileURLToPath(new URL("../assets/index.html", import.meta.url)), {
      force: true,
    });
  },
};

// The bundle is committed: `cargo build` must never need bun. That makes byte
// stability part of the contract, so hashed filenames are off.
export default defineConfig({
  plugins: [svelte(), dropShell],
  build: {
    outDir: "../assets",
    emptyOutDir: false,
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        entryFileNames: "chat.js",
        // Deterministic chunk names, because the Rust server embeds every file
        // by name: a hashed chunk would be a 404. `chunk-[name].js` for the same
        // reason the entry is `chat.js` — the bundle is committed.
        chunkFileNames: "chunk-[name].js",
        assetFileNames: "chat.[ext]",
      },
    },
  },
});
