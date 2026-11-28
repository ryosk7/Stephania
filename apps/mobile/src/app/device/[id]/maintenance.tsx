import {useLocalSearchParams} from 'expo-router';
import {MaintenanceScreen} from '@/features/maintenance';

export default function Route() {
  const {id} = useLocalSearchParams<{id: string}>();
  return <MaintenanceScreen deviceId={id} />;
}
