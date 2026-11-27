import type {Preview} from '@storybook/react-vite';
import '../src/styles.css';
import {initAppearance} from '../src/theme';
import {ThemeSwitch} from '../src/theme-switch';

initAppearance();

const preview: Preview = {
  parameters: {layout: 'fullscreen'},
  decorators: [Story => (
    <div className="min-h-screen bg-bg p-4 text-text">
      <div className="mb-4 flex justify-end"><ThemeSwitch/></div>
      <div data-testid="story"><Story/></div>
    </div>
  )],
};

export default preview;
