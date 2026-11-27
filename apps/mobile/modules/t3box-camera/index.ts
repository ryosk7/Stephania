// Native camera sender (docs/PHONE.md "Phone app sender"). iOS today; the
// contract in src/T3BoxCamera.types.ts is what an Android implementation
// must provide.
export {default, CameraPreview} from './src/T3BoxCameraModule';
export * from './src/T3BoxCamera.types';
