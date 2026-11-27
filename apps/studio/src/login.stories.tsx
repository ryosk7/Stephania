import type {Meta, StoryObj} from '@storybook/react-vite';
import {Login} from './login';

const meta = {title: 'Shell/Sign in', component: Login, args: {onToken: () => {}, onLogin: () => {}, busy: false}} satisfies Meta<typeof Login>;
export default meta;
export const SignIn: StoryObj<typeof meta> = {};
