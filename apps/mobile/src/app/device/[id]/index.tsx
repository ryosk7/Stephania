import {useLocalSearchParams} from 'expo-router';
import {ControlScreen} from '@/features/control';

export default function Route() {
  const {id} = useLocalSearchParams<{id: string}>();
  return <ControlScreen deviceId={id} />;
}
