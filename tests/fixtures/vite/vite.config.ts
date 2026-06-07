import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// CoreGraph test fixture — Vite + React project
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: 'dist',
  },
  server: {
    port: 3000,
  },
})
