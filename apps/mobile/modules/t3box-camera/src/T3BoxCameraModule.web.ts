// The camera sender is native only; on web nothing is exported that works.
import type {NativeCamera} from './T3BoxCamera.types';
const unsupported = () => Promise.reject(new Error('camera sender is not available on this platform'));
const web: NativeCamera = {
  capabilities: async () => ({roles: [], dualProfiles: [], hardwareH264: false}),
  thermalState: () => 'nominal',
  startCapture: unsupported, stopCapture: async () => {}, createOffer: unsupported, setAnswer: unsupported,
  applyPlans: async () => {}, closePeer: async () => {},
  addListener: () => ({remove() {}}),
};
export default web;
export const CameraPreview: (props: import('./T3BoxCameraModule').CameraPreviewProps) => null = () => null;
