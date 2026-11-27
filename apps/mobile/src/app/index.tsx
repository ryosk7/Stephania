import {HomeScreen} from '@/features/home';
import {useSetupSheet} from '@/features/setup';

export default function Home() {
  return <HomeScreen useSetup={useSetupSheet} />;
}
