// Web Storybook for the same colocated stories (react-native-web + Vite):
// `npm run storybook:web`, and `npm run build-storybook` in CI, so every
// component renders in light and dark without a phone.
import {fileURLToPath} from 'node:url';
import type {StorybookConfig} from '@storybook/react-native-web-vite';
import tailwindcss from '@tailwindcss/vite';
import {uniwind} from 'uniwind/vite';
import {mergeConfig} from 'vite';

const src = fileURLToPath(new URL('../src/', import.meta.url));
// react-native-svg's web build asks for React Native's asset registry; on the web it is react-native-web's.
const assetRegistry = fileURLToPath(new URL('../node_modules/react-native-web/dist/modules/AssetRegistry/index.js', import.meta.url));

/** @expo-google-fonts load their .ttf with require(), which Metro resolves; Vite needs asset URLs. */
const fontRequires = {
  name: 't3box-expo-font-require',
  transform(code: string, id: string) {
    if (!id.includes('@expo-google-fonts')) return null;
    return code.replace(/require\((['"][^'"]+\.ttf['"])\)/g, 'new URL($1, import.meta.url).href');
  },
};

const config: StorybookConfig = {
  stories: ['../src/**/*.stories.@(ts|tsx)'],
  framework: {name: '@storybook/react-native-web-vite', options: {}},
  viteFinal: c => mergeConfig(c, {
    plugins: [fontRequires, tailwindcss(), uniwind({cssEntryFile: './src/global.css', dtsFile: './src/uniwind-types.d.ts'})],
    resolve: {alias: {'@/': src, '@react-native/assets-registry/registry': assetRegistry}},
  }),
};

export default config;
