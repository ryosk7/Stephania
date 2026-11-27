import type {StorybookConfig} from '@storybook/react-native';

// Stories live next to their components (src/**/name.stories.tsx).
const main: StorybookConfig = {
  stories: ['../src/**/*.stories.?(ts|tsx)'],
  deviceAddons: ['@storybook/addon-ondevice-actions'],
};

export default main;
