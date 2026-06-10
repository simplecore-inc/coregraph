import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { viteSingleFile } from 'vite-plugin-singlefile'

export default defineConfig({
  plugins: [react(), viteSingleFile()],
  build: {
    // The bundle intentionally inlines three.js and the embedded graph data.
    chunkSizeWarningLimit: 16384,
  },
})
