import type {Meta, StoryObj} from '@storybook/react-native';
import {AppearancePicker} from './appearance-picker';

const meta = {title: 'Settings/Appearance', component: AppearancePicker} satisfies Meta<typeof AppearancePicker>;
export default meta;

export const Picker: StoryObj<typeof meta> = {};
