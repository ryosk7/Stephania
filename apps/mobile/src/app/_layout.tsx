import {Stack} from 'expo-router';

export default function Layout() {
  return (
    <Stack screenOptions={{headerTitleAlign: 'center'}}>
      <Stack.Screen name="index" options={{title: 'T3Box Connect'}} />
      <Stack.Screen name="scan" options={{title: 'セットアップQRを読む'}} />
      <Stack.Screen name="devices" options={{title: '近くのT3Box'}} />
      <Stack.Screen name="wifi" options={{title: 'Wi-Fi設定'}} />
    </Stack>
  );
}
