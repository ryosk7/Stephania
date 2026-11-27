import {requireNativeView, requireOptionalNativeModule} from 'expo';
import type {ComponentType} from 'react';
import type {ViewProps} from 'react-native';
import type {CameraRole, NativeCamera} from './StephaniaCamera.types';
import unsupported from './StephaniaCameraModule.web';

// `camera`, not `role`: ViewProps already has an accessibility `role`.
export type CameraPreviewProps = ViewProps & {camera: CameraRole; mirrored?: boolean};

// Optional: platforms without the native sender yet (Android) get the
// stub, whose capabilities() reports no cameras.
const native = requireOptionalNativeModule<NativeCamera>('StephaniaCamera');
export default (native ?? unsupported) as NativeCamera;

/** Live preview of one captured camera, rendered natively (no frames cross the JS bridge). */
export const CameraPreview: ComponentType<CameraPreviewProps> =
  native ? requireNativeView<CameraPreviewProps>('StephaniaCamera') : () => null;
