import {NativeModule, requireNativeModule} from 'expo';

export type NativeResponse = {status: number; headers: string; body: string};

declare class T3BoxPinnedFetchModule extends NativeModule<{}> {
  request(url: string, method: string, headersJson: string, body: string | null, caSha256: string, timeoutMs: number): Promise<NativeResponse>;
}

export default requireNativeModule<T3BoxPinnedFetchModule>('T3BoxPinnedFetch');
