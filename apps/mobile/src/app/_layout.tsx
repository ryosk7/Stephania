// Root navigator (routes only; screens live in src/features, DESIGN.md §7).
// Headers are drawn by the screens (TopBar), not by the platform navigator,
// so iOS and Android look the same. The saved appearance (system by
// default) is applied before the first screen, so nothing flashes.
import '../global.css';
import {Stack} from 'expo-router';
import * as SplashScreen from 'expo-splash-screen';
import {StatusBar} from 'expo-status-bar';
import {useEffect, useState} from 'react';
import {restoreAppearance, useAppFonts, useColor} from '@/shared/ui';

void SplashScreen.preventAutoHideAsync();

/** Storybook (npm run storybook) is compiled into development builds only. */
const storybook = __DEV__ && process.env.EXPO_PUBLIC_STORYBOOK_ENABLED === 'true';

export default function Layout() {
  const [fonts, fontError] = useAppFonts();
  const [themed, setThemed] = useState(false);
  useEffect(() => { void restoreAppearance().finally(() => setThemed(true)); }, []);
  const ready = (fonts || !!fontError) && themed;
  useEffect(() => { if (ready) void SplashScreen.hideAsync(); }, [ready]);
  const bg = useColor('bg');
  if (!ready) return null;
  return (
    <>
      <StatusBar style="auto" />
      <Stack screenOptions={{headerShown: false, contentStyle: {backgroundColor: bg}, animation: 'slide_from_right'}}>
        <Stack.Screen name="device/[id]/camera" options={{presentation: 'fullScreenModal', animation: 'slide_from_bottom'}} />
        <Stack.Protected guard={storybook}>
          <Stack.Screen name="storybook" />
        </Stack.Protected>
      </Stack>
    </>
  );
}
