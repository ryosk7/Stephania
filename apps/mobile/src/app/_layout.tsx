// Root navigator (routes only; screens live in src/features, DESIGN.md §7).
// Headers are drawn by the screens (TopBar), not by the platform navigator,
// so iOS and Android look the same.
import {Stack} from 'expo-router';
import * as SplashScreen from 'expo-splash-screen';
import {StatusBar} from 'expo-status-bar';
import {useEffect} from 'react';
import {color, useAppFonts} from '@/shared/ui';

void SplashScreen.preventAutoHideAsync();

export default function Layout() {
  const [loaded, error] = useAppFonts();
  useEffect(() => { if (loaded || error) void SplashScreen.hideAsync(); }, [loaded, error]);
  if (!loaded && !error) return null;
  return (
    <>
      <StatusBar style="light" />
      <Stack screenOptions={{headerShown: false, contentStyle: {backgroundColor: color.bg}, animation: 'slide_from_right'}}>
        <Stack.Screen name="device/[id]/camera" options={{presentation: 'fullScreenModal', animation: 'slide_from_bottom'}} />
      </Stack>
    </>
  );
}
