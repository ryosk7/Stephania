import type {Meta, StoryObj} from '@storybook/react-vite';
import {Meter} from './audio';

const meta = {title: 'Audio/Meter', component: Meter, args: {label: 'mic', level: {rms_dbfs: -18, peak_dbfs: -6, clipped: false}}} satisfies Meta<typeof Meter>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Speech: Story = {};
export const Clipping: Story = {args: {level: {rms_dbfs: -3, peak_dbfs: 0, clipped: true}}};
