import { defineConfig } from 'vite';

// Tauri公式Viteテンプレート相当の設定。詳細はCLAUDE.md「アプリ起動」節参照。
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    target: 'es2022',
  },
});
