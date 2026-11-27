import {useLocalSearchParams} from 'expo-router';
import {PcAccessScreen} from '@/features/pc-access';

export default function Route() {
  const {id} = useLocalSearchParams<{id: string}>();
  return <PcAccessScreen deviceId={id} />;
}
