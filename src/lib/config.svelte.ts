import * as ipc from './ipc';
import { notify } from './notify.svelte';
import { defaultConfig, sanitizeConfig, type Config } from './types';

/** Reactive, persisted user configuration. Edits apply immediately and are saved debounced. */
class ConfigStore {
  value = $state<Config>(defaultConfig());
  path = $state('');
  warning = $state<string | null>(null);
  saveError = $state<string | null>(null);
  loaded = $state(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  async load() {
    try {
      const p = await ipc.getConfig();
      this.value = sanitizeConfig(p.config);
      this.path = p.path;
      this.warning = p.warning;
    } catch (e) {
      this.warning = `Could not load configuration: ${String(e)}. Using defaults.`;
    }
    this.loaded = true;
  }

  /** Apply a mutation and schedule persistence. */
  update(mutate: (c: Config) => void) {
    const next = structuredClone($state.snapshot(this.value)) as Config;
    mutate(next);
    this.value = sanitizeConfig(next);
    this.schedule();
  }

  reset() {
    this.value = defaultConfig();
    this.schedule(0);
  }

  private schedule(delay = 300) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), delay);
  }

  async flush() {
    clearTimeout(this.timer);
    try {
      this.value = sanitizeConfig(await ipc.saveConfig($state.snapshot(this.value) as Config));
      this.saveError = null;
      this.warning = null;
    } catch (e) {
      this.saveError = String(e);
      // Told once per failure streak: the page keeps showing the error until a save works.
      notify.error('Your settings could not be saved.', 'settings', String(e));
    }
  }
}

export const config = new ConfigStore();
