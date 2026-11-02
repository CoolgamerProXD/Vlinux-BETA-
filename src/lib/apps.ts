// Application registry (metadata only — components are mapped in components/AppHost.svelte).
//
// `status: 'planned'` entries are shown disabled and labelled with the milestone that
// delivers them. They are never clickable, so there are no buttons that do nothing.

import type { IconName } from './icons';

export interface AppDef {
  id: string;
  name: string;
  description: string;
  icon: IconName;
  /** Tile tint (CSS colour). */
  color: string;
  status: 'available' | 'planned';
  /** Milestone that delivers a planned app. */
  milestone?: string;
  size: { w: number; h: number };
  /** Shown in the dock. */
  dock: boolean;
}

export const APPS: AppDef[] = [
  { id: 'terminal', color: '#3ddc97', name: 'Terminal', description: 'A real interactive shell inside your Linux distribution', icon: 'terminal', status: 'available', size: { w: 860, h: 520 }, dock: true },
  { id: 'files', color: '#ffb454', name: 'Files', description: 'Browse your Linux files and shared Windows folders', icon: 'folder', status: 'available', size: { w: 900, h: 600 }, dock: true },
  { id: 'distros', color: '#5ee0ff', name: 'Distributions', description: 'See, inspect, start and configure your WSL distributions', icon: 'layers', status: 'available', size: { w: 920, h: 620 }, dock: true },
  { id: 'setup', color: '#7cf2c6', name: 'Linux Setup', description: 'Detect WSL, choose or install a distribution', icon: 'cpu', status: 'available', size: { w: 800, h: 600 }, dock: true },
  { id: 'system', color: '#ff7aa8', name: 'System', description: 'Host status, WSL readiness and project roadmap', icon: 'activity', status: 'available', size: { w: 700, h: 540 }, dock: true },
  { id: 'settings', color: '#a98bff', name: 'Settings', description: 'Appearance, desktop and startup options', icon: 'sliders', status: 'available', size: { w: 820, h: 560 }, dock: true },
];

export const appById = (id: string): AppDef | undefined => APPS.find((a) => a.id === id);
