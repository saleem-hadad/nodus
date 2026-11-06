import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '@xyflow/react/dist/style.css'
import '@xterm/xterm/css/xterm.css'
import './styles/global.css'
import './styles/app.css'
import { App } from './App'
import { platform } from './platform'

// Off a Mac the title bar leaves room for Windows' buttons rather than the traffic lights.
document.documentElement.dataset.platform = platform

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
