import { resolve } from 'path'
import { defineConfig } from 'vitest/config'

// Unit tests (`pnpm test`): *.test.ts beside the code they test, run in Node.
export default defineConfig({
  resolve: {
    alias: {
      '@shared': resolve('src/shared'),
      '@renderer': resolve('src/renderer/src')
    }
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node'
  }
})
