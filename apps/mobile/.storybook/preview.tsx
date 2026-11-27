import type {Preview} from '@storybook/react-native-web-vite';
import '../src/global.css';
import {themeDecorator} from '../src/shared/ui/storybook';

const preview: Preview = {decorators: [themeDecorator], parameters: {layout: 'fullscreen'}};

export default preview;
