import type {Preview} from '@storybook/react-native';
import {themeDecorator} from '../src/shared/ui/storybook';

const preview: Preview = {decorators: [themeDecorator]};

export default preview;
