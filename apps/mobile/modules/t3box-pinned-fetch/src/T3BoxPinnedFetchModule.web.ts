import {NativeModule, registerWebModule} from 'expo';

// Pinned TLS needs the native trust APIs; the web build has none.
class T3BoxPinnedFetchModule extends NativeModule<{}> {
  async request(): Promise<never> {
    throw new Error('pinned requests are not available on the web');
  }
}

export default registerWebModule(T3BoxPinnedFetchModule, 'T3BoxPinnedFetchModule');
