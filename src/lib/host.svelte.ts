import * as ipc from './ipc';
import type { SystemInfo } from './types';

interface BatteryManagerLike extends EventTarget {
  level: number;
  charging: boolean;
}

/**
 * Host (Windows) state shown in the bar. Everything is event-driven — there is no
 * polling timer: full-screen comes from a Rust event, network from online/offline
 * events and battery from the Battery Status API events.
 */
class Host {
  fullscreen = $state(false);
  online = $state(typeof navigator === 'undefined' ? true : navigator.onLine);
  battery = $state<{ level: number; charging: boolean } | null>(null);
  info = $state<SystemInfo | null>(null);
  private disposers: Array<() => void> = [];

  async init() {
    this.info = await ipc.getSystemInfo().catch(() => null);
    this.fullscreen = await ipc.isFullscreen().catch(() => false);
    this.disposers.push(await ipc.onFullscreenChange((on) => (this.fullscreen = on)));

    const on = () => (this.online = true);
    const off = () => (this.online = false);
    window.addEventListener('online', on);
    window.addEventListener('offline', off);
    this.disposers.push(() => {
      window.removeEventListener('online', on);
      window.removeEventListener('offline', off);
    });

    const nav = navigator as Navigator & { getBattery?: () => Promise<BatteryManagerLike> };
    if (nav.getBattery) {
      try {
        const b = await nav.getBattery();
        const sync = () => (this.battery = { level: b.level, charging: b.charging });
        sync();
        b.addEventListener('levelchange', sync);
        b.addEventListener('chargingchange', sync);
        this.disposers.push(() => {
          b.removeEventListener('levelchange', sync);
          b.removeEventListener('chargingchange', sync);
        });
      } catch {
        /* no battery information available — leave null */
      }
    }
  }

  dispose() {
    this.disposers.forEach((d) => d());
    this.disposers = [];
  }
}

export const host = new Host();
