// Three-way appearance switch (system / light / dark) as icons; the words
// are the accessible names and tooltips.
import {Moon, Sun, SunMoon} from 'lucide-react';
import {appearanceLabel, appearances, type Appearance} from './appearance';
import {setAppearance, useAppearance} from './theme';

const icon = {system: SunMoon, light: Sun, dark: Moon} satisfies Record<Appearance, unknown>;

export function ThemeSwitch() {
  const current = useAppearance();
  return (
    <div role="radiogroup" aria-label="Appearance" className="flex gap-0.5 rounded-button border border-border bg-surface p-0.5">
      {appearances.map(a => {
        const I = icon[a];
        return (
          <button key={a} role="radio" aria-checked={a === current} aria-label={appearanceLabel[a]} title={appearanceLabel[a]}
            onClick={() => setAppearance(a)}
            className={`m-0 flex h-8 w-8 items-center justify-center rounded-control border-0 p-0 ${a === current ? 'bg-text text-on-primary hover:bg-text' : 'bg-transparent text-secondary'}`}>
            <I size={16} aria-hidden/>
          </button>
        );
      })}
    </div>
  );
}
