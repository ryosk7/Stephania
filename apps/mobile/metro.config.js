// Metro: Tailwind CSS v4 through Uniwind (src/global.css) and Storybook for
// React Native. Storybook is compiled out unless EXPO_PUBLIC_STORYBOOK_ENABLED
// is "true" (npm run storybook); Uniwind must be the outermost wrapper.
const {getDefaultConfig} = require('expo/metro-config');
const {withStorybook} = require('@storybook/react-native/metro/withStorybook');
const {withUniwindConfig} = require('uniwind/metro');

const config = getDefaultConfig(__dirname);

module.exports = withUniwindConfig(
  withStorybook(config, {enabled: process.env.EXPO_PUBLIC_STORYBOOK_ENABLED === 'true', configPath: './.rnstorybook'}),
  {cssEntryFile: './src/global.css', dtsFile: './src/uniwind-types.d.ts'},
);
