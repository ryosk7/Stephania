import {HomeScreen} from '@/features/home';
import {useSetupSheet} from '@/features/setup';
import {useSplashGone} from '@/shared/ui';

/** The first-run setup sheet opens only once the launch splash has gone. */
function useSetupAfterSplash(firstRun: boolean) {
  const splashGone = useSplashGone();
  return useSetupSheet(firstRun && splashGone);
}

export default function Home() {
  return <HomeScreen useSetup={useSetupAfterSplash} />;
}
