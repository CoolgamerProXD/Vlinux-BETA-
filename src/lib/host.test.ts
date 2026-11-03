import { beforeEach, describe, expect, it, vi } from 'vitest';

// Regression (found in the real WebView2 window): after leaving full screen, F11 and the top-bar
// button did nothing because the toggle used a cached flag that the window's change event never
// refreshed. The toggle must follow what the window REALLY is.
const state = { real: true, calls: [] as boolean[] };
vi.mock('./ipc', () => ({
  getSystemInfo: async () => null,
  isFullscreen: async () => state.real,
  setFullscreen: async (on: boolean) => {
    state.calls.push(on);
    state.real = on;
    return on;
  },
  onFullscreenChange: async () => () => {},
}));

import { host } from './host.svelte';

describe('host.toggleFullscreen', () => {
  beforeEach(() => {
    state.real = true;
    state.calls = [];
  });

  it('asks for the opposite of the real window state even when the cached flag is stale', async () => {
    host.fullscreen = true; // stale: the window is in fact windowed
    state.real = false;
    await host.toggleFullscreen();
    expect(state.calls).toEqual([true]);
    expect(host.fullscreen).toBe(true);
  });

  it('leaves full screen when the window is full screen, and the flag follows', async () => {
    host.fullscreen = false; // stale the other way
    state.real = true;
    await host.toggleFullscreen();
    expect(state.calls).toEqual([false]);
    expect(host.fullscreen).toBe(false);
  });

  it('toggles back and forth without drifting', async () => {
    state.real = false;
    for (let i = 0; i < 4; i++) await host.toggleFullscreen();
    expect(state.calls).toEqual([true, false, true, false]);
    expect(state.real).toBe(false);
  });
});
