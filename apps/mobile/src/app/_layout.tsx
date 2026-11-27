// Root navigator (routes only; screens live in src/features, DESIGN.md §7).
import {Stack, ThemeProvider} from 'expo-router';
import {StatusBar} from 'expo-status-bar';
import {navigationTheme, stackScreenOptions} from '@/shared/ui';

export default function Layout() {
  return (
    <ThemeProvider value={navigationTheme}>
      <StatusBar style="light" />
      <Stack screenOptions={stackScreenOptions}>
        <Stack.Screen name="index" options={{title: '', headerShown: false}} />
        <Stack.Screen name="setup/scan" options={{title: 'Set up'}} />
        <Stack.Screen name="setup/nearby" options={{title: 'Set up'}} />
        <Stack.Screen name="setup/wifi" options={{title: 'Wi-Fi'}} />
        <Stack.Screen name="device/[id]/index" options={{title: ''}} />
        <Stack.Screen name="device/[id]/camera" options={{headerShown: false, presentation: 'fullScreenModal'}} />
        <Stack.Screen name="device/[id]/pc" options={{title: 'Open on PC'}} />
      </Stack>
    </ThemeProvider>
  );
}
