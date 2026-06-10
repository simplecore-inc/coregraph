import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '@fontsource-variable/martian-mono'
import './styles.css'
import App from './App.tsx'

const container = document.getElementById('root')
if (container === null) {
  throw new Error('missing #root element')
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
