import type {StorybookConfig} from '@storybook/react-vite';

// Stories live next to their components (src/**/name.stories.tsx).
const config: StorybookConfig = {
  stories: ['../src/**/*.stories.@(ts|tsx)'],
  framework: {name: '@storybook/react-vite', options: {}},
};

export default config;
