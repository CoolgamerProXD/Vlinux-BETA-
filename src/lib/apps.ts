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
  { id: 'terminal', color: '#3ddc97', name: 'Terminal', description: 'Interactive Linux shell (real pseudoterminal)', icon: 'terminal', status: 'planned', milestone: 'M3', size: { w: 760, h: 480 }, dock: true },
  { id: 'files', color: '#ffb454', name: 'Files', description: 'Browse the Linux filesystem', icon: 'folder', status: 'planned', milestone: 'M4', size: { w: 820, h: 520 }, dock: true },
  { id: 'distros', color: '#5ee0ff', name: 'Distributions', description: 'Install and manage Linux distributions', icon: 'layers', status: 'planned', milestone: 'M5', size: { w: 860, h: 560 }, dock: false },
  { id: 'system', color: '#ff7aa8', name: 'System', description: 'Host status, WSL readiness and project roadmap', icon: 'activity', status: 'available', size: { w: 700, h: 540 }, dock: true },
  { id: 'settings', color: '#a98bff', name: 'Settings', description: 'Appearance, desktop and startup options', icon: 'sliders', status: 'available', size: { w: 820, h: 560 }, dock: true },
];

export const appById = (id: string): AppDef | undefined => APPS.find((a) => a.id === id);
