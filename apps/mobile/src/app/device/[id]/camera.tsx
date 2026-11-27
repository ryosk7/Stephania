import {useLocalSearchParams} from 'expo-router';
import {CameraScreen} from '@/features/camera';

export default function Route() {
  const {id} = useLocalSearchParams<{id: string}>();
  return <CameraScreen deviceId={id} />;
}
