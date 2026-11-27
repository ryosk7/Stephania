// Navigation entries: path, label (the accessible name) and icon.
import {Gauge, LayoutTemplate, Settings, SlidersVertical, type LucideIcon} from 'lucide-react';

export const pages = [
  {to: '/', label: 'Dashboard', icon: Gauge},
  {to: '/studio', label: 'Studio', icon: LayoutTemplate},
  {to: '/audio', label: 'Audio', icon: SlidersVertical},
  {to: '/settings', label: 'Settings', icon: Settings},
] as const satisfies readonly {to: string; label: string; icon: LucideIcon}[];
