import {Stack} from 'expo-router';

export default function Layout() {
  return (
    <Stack screenOptions={{headerTitleAlign: 'center'}}>
      <Stack.Screen name="index" options={{title: 'T3Box Connect'}} />
      <Stack.Screen name="scan" options={{title: 'Scan setup QR'}} />
      <Stack.Screen name="devices" options={{title: 'Nearby T3Box'}} />
      <Stack.Screen name="wifi" options={{title: 'Wi-Fi setup'}} />
      <Stack.Screen name="dashboard" options={{title: 'Control'}} />
      <Stack.Screen name="open-on-pc" options={{title: 'Open on PC'}} />
      <Stack.Screen name="camera" options={{title: 'Camera'}} />
    </Stack>
  );
}
