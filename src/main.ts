import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

// Block the browser/WebView context menu: VLINUX is an app shell, not a web page.
window.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement | null;
  if (!t?.closest('input, textarea, [contenteditable], code')) e.preventDefault();
});

export default mount(App, { target: document.getElementById('app')! });
