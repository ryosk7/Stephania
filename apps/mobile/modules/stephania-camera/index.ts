// Native camera sender (docs/PHONE.md "Phone app sender"). iOS today; the
// contract in src/StephaniaCamera.types.ts is what an Android implementation
// must provide.
export {default, CameraPreview} from './src/StephaniaCameraModule';
export * from './src/StephaniaCamera.types';
