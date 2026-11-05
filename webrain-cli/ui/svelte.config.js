// The Svelte language server refuses to type-check a component until it finds
// this file ("No Svelte configuration found in vite config"), even though the
// build works without it. Runes are explicit so a stray legacy component fails
// here instead of silently compiling in the old mode.
export default {
  compilerOptions: { runes: true },
};
