// Fonts bundled with the app (OFL): Inter for text, JetBrains Mono for
// addresses and fingerprints. Loaded once in the root layout.
import {Inter_400Regular, Inter_500Medium, Inter_600SemiBold, Inter_700Bold} from '@expo-google-fonts/inter';
import {JetBrainsMono_500Medium} from '@expo-google-fonts/jetbrains-mono';
import {useFonts} from 'expo-font';

export const useAppFonts = () => useFonts({Inter_400Regular, Inter_500Medium, Inter_600SemiBold, Inter_700Bold, JetBrainsMono_500Medium});
