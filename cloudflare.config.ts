import { defineConfig } from "cf/config";

// Assets-only Worker: the files in web/ are served as they are.
// Deploy with `npm run deploy` (= `cf deploy web`).
export default defineConfig({
  worker: {
    name: "physical-modeling-piano-f1",
    compatibilityDate: "2026-11-01",
    domains: ["piano.yoshiyukikoyanagi.com"],
    workersDev: false,
    previewUrls: false,
  },
});
