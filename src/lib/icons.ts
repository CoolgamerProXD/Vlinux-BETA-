// Original, hand-drawn 24×24 stroke icons (no third-party icon set).
export type IconName =
  | 'terminal' | 'folder' | 'layers' | 'activity' | 'sliders' | 'power' | 'search' | 'grid'
  | 'wifi' | 'wifi-off' | 'battery' | 'expand' | 'shrink' | 'close' | 'minus' | 'square'
  | 'windows' | 'check' | 'palette' | 'monitor' | 'keyboard' | 'info' | 'cpu' | 'alert' | 'rotate';

export const ICONS: Record<IconName, string> = {
  terminal: 'M3.5 5.5h17a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1zM7 10l3 2.5L7 15M12.5 15H17',
  folder: 'M3 7.5a1.5 1.5 0 0 1 1.5-1.5h4.3l2 2.2h8.7A1.5 1.5 0 0 1 21 9.7v7.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z',
  layers: 'M12 3.5l9 4.8-9 4.8-9-4.8zM3.5 12.2L12 16.7l8.5-4.5M3.5 16l8.5 4.5 8.5-4.5',
  activity: 'M3 12h4l2.5-6.5L14 18l2.5-6H21',
  sliders: 'M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5',
  power: 'M12 3.5v8M7.2 6.6a7.5 7.5 0 1 0 9.6 0',
  search: 'M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.5 15.5L20 20',
  grid: 'M4.5 4.5h6v6h-6zM13.5 4.5h6v6h-6zM4.5 13.5h6v6h-6zM13.5 13.5h6v6h-6z',
  wifi: 'M3 9.5a14 14 0 0 1 18 0M6 13a9.5 9.5 0 0 1 12 0M9 16.5a5 5 0 0 1 6 0M12 20h.01',
  'wifi-off': 'M3 3l18 18M3 9.5a14 14 0 0 1 4-2.4M10 5.2A14 14 0 0 1 21 9.5M6 13a9.5 9.5 0 0 1 3-1.8M14.5 11.4A9.5 9.5 0 0 1 18 13M9 16.5a5 5 0 0 1 6 0M12 20h.01',
  battery: 'M3 9a1.5 1.5 0 0 1 1.5-1.5h13A1.5 1.5 0 0 1 19 9v6a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 15zM21.5 10.5v3M6 10.5v3',
  expand: 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5',
  shrink: 'M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5',
  close: 'M6 6l12 12M18 6L6 18',
  minus: 'M6 12h12',
  square: 'M6.5 6.5h11v11h-11z',
  windows: 'M3.5 5.8l7.2-1v6.7H3.5zM12 4.6l8.5-1.2v8.1H12zM3.5 12.6h7.2v6.7l-7.2-1zM12 12.6h8.5v8L12 19.4z',
  check: 'M5 12.5l4.5 4.5L19 7.5',
  palette: 'M12 3.5a8.5 8.5 0 1 0 0 17c1.2 0 1.8-.8 1.8-1.6 0-1.4-1.3-1.6-1.3-2.7 0-.9.7-1.5 1.7-1.5H17a3.5 3.5 0 0 0 3.5-3.5C20.5 6.6 16.8 3.5 12 3.5zM8 11h.01M11 7.8h.01M15 8.5h.01',
  monitor: 'M3.5 5h17a1 1 0 0 1 1 1v9.5a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM8.5 20h7M12 16.5V20',
  keyboard: 'M3.5 6.5h17a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zM6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M7.5 14.5h9',
  info: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 11v5.5M12 7.8h.01',
  cpu: 'M7 7h10v10H7zM10 10h4v4h-4zM9.5 3.5v3.5M14.5 3.5v3.5M9.5 17v3.5M14.5 17v3.5M3.5 9.5H7M3.5 14.5H7M17 9.5h3.5M17 14.5h3.5',
  alert: 'M12 4l9 15.5H3zM12 10v4.5M12 17.2h.01',
  rotate: 'M4 12a8 8 0 0 1 14-5.3L20 9M20 4v5h-5M20 12a8 8 0 0 1-14 5.3L4 15M4 20v-5h5',
};
