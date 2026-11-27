import type {Meta, StoryObj} from '@storybook/react-vite';
import {ThemeSwitch} from './theme-switch';

const meta = {title: 'Shell/Theme switch', component: ThemeSwitch} satisfies Meta<typeof ThemeSwitch>;
export default meta;
export const Switch: StoryObj<typeof meta> = {};
